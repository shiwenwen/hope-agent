#![allow(non_snake_case)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{
    define_class, extern_class, extern_methods, extern_protocol, msg_send, AnyThread, DefinedClass,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use tokio::sync::oneshot;

#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}

type CLAuthorizationStatus = isize;
type CLLocationAccuracy = f64;
type LocationRequestResult = Result<(f64, f64), String>;
type LocationCompletion = Arc<Mutex<Option<oneshot::Sender<LocationRequestResult>>>>;
type LocationRequestFlowHandle = Arc<Mutex<LocationRequestFlow>>;

const CL_AUTHORIZATION_STATUS_NOT_DETERMINED: CLAuthorizationStatus = 0;
const CL_AUTHORIZATION_STATUS_RESTRICTED: CLAuthorizationStatus = 1;
const CL_AUTHORIZATION_STATUS_DENIED: CLAuthorizationStatus = 2;
const CL_AUTHORIZATION_STATUS_AUTHORIZED_ALWAYS: CLAuthorizationStatus = 3;
const CL_AUTHORIZATION_STATUS_AUTHORIZED_WHEN_IN_USE: CLAuthorizationStatus = 4;
const KCLLOCATION_ACCURACY_KILOMETER: CLLocationAccuracy = 1000.0;
const KCLERROR_LOCATION_UNKNOWN: isize = 0;
const LOCATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(40);

static NEXT_LOCATION_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    // CoreLocation delegates fire on the manager's run loop; we create and retain
    // each request on the main thread so callbacks can complete it without polling.
    static ACTIVE_LOCATION_REQUESTS: RefCell<HashMap<u64, ActiveLocationRequest>> =
        RefCell::new(HashMap::new());
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CLLocationCoordinate2D {
    latitude: f64,
    longitude: f64,
}

unsafe impl Encode for CLLocationCoordinate2D {
    const ENCODING: Encoding =
        Encoding::Struct("CLLocationCoordinate2D", &[f64::ENCODING, f64::ENCODING]);
}

unsafe impl RefEncode for CLLocationCoordinate2D {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

extern_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct CLLocationManager;
);

impl CLLocationManager {
    extern_methods!(
        #[unsafe(method(init))]
        #[unsafe(method_family = init)]
        fn init(this: Allocated<Self>) -> Retained<Self>;

        #[unsafe(method(new))]
        #[unsafe(method_family = new)]
        fn new() -> Retained<Self>;

        #[unsafe(method(authorizationStatus))]
        #[unsafe(method_family = none)]
        fn authorizationStatus() -> CLAuthorizationStatus;

        #[unsafe(method(setDelegate:))]
        #[unsafe(method_family = none)]
        unsafe fn setDelegate(
            &self,
            delegate: Option<&ProtocolObject<dyn CLLocationManagerDelegate>>,
        );

        #[unsafe(method(setDesiredAccuracy:))]
        #[unsafe(method_family = none)]
        fn setDesiredAccuracy(&self, desired_accuracy: CLLocationAccuracy);

        #[unsafe(method(requestLocation))]
        #[unsafe(method_family = none)]
        fn requestLocation(&self);
    );
}

extern_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct CLLocation;
);

impl CLLocation {
    extern_methods!(
        #[unsafe(method(coordinate))]
        #[unsafe(method_family = none)]
        fn coordinate(&self) -> CLLocationCoordinate2D;
    );
}

extern_protocol!(
    unsafe trait CLLocationManagerDelegate: NSObjectProtocol {
        #[optional]
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        #[unsafe(method_family = none)]
        fn locationManagerDidChangeAuthorization(&self, manager: &CLLocationManager);

        #[unsafe(method(locationManager:didUpdateLocations:))]
        #[unsafe(method_family = none)]
        fn locationManager_didUpdateLocations(
            &self,
            manager: &CLLocationManager,
            locations: &NSArray<CLLocation>,
        );

        #[unsafe(method(locationManager:didFailWithError:))]
        #[unsafe(method_family = none)]
        fn locationManager_didFailWithError(&self, manager: &CLLocationManager, error: &NSError);
    }
);

#[derive(Debug, Default)]
struct LocationRequestFlow {
    prompted_for_authorization: bool,
    requested_authorized_location: bool,
}

struct ActiveLocationRequest {
    manager: Retained<CLLocationManager>,
    _delegate: Retained<LocationDelegate>,
}

#[derive(Debug, Clone)]
struct LocationDelegateIvars {
    request_id: u64,
    flow: LocationRequestFlowHandle,
    completion: LocationCompletion,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[ivars = LocationDelegateIvars]
    struct LocationDelegate;

    unsafe impl NSObjectProtocol for LocationDelegate {}

    unsafe impl CLLocationManagerDelegate for LocationDelegate {
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn locationManagerDidChangeAuthorization(&self, manager: &CLLocationManager) {
            let status = CLLocationManager::authorizationStatus();

            if is_authorized(status) {
                request_authorized_location_if_needed(
                    manager,
                    &self.ivars().flow,
                    "authorization callback",
                );
            } else if is_denied_or_restricted(status) {
                finish_location_request(
                    self.ivars().request_id,
                    manager,
                    &self.ivars().completion,
                    Err(format!(
                        "CoreLocation unavailable: status={} {}",
                        status,
                        authorization_status_name(status)
                    )),
                );
            }
        }

        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn locationManager_didUpdateLocations(
            &self,
            manager: &CLLocationManager,
            locations: &NSArray<CLLocation>,
        ) {
            let Some(location) = locations.lastObject() else {
                finish_location_request(
                    self.ivars().request_id,
                    manager,
                    &self.ivars().completion,
                    Err("CoreLocation returned no locations".to_string()),
                );
                return;
            };

            let coord = location.coordinate();
            finish_location_request(
                self.ivars().request_id,
                manager,
                &self.ivars().completion,
                Ok((coord.latitude, coord.longitude)),
            );
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn locationManager_didFailWithError(&self, manager: &CLLocationManager, error: &NSError) {
            let status = CLLocationManager::authorizationStatus();
            let code = error.code();

            if status == CL_AUTHORIZATION_STATUS_NOT_DETERMINED
                || (is_authorized(status) && code == KCLERROR_LOCATION_UNKNOWN)
            {
                crate::app_debug!(
                    "weather",
                    "system_locate",
                    "Ignoring transient CoreLocation error while waiting: code={} status={} {} desc={}",
                    code,
                    status,
                    authorization_status_name(status),
                    crate::truncate_utf8(&error.localizedDescription().to_string(), 300)
                );
                return;
            }

            finish_location_request(
                self.ivars().request_id,
                manager,
                &self.ivars().completion,
                Err(format!(
                    "CoreLocation failed: code={} status={} {} desc={}",
                    code,
                    status,
                    authorization_status_name(status),
                    crate::truncate_utf8(&error.localizedDescription().to_string(), 300)
                )),
            );
        }
    }
);

impl LocationDelegate {
    fn new(
        request_id: u64,
        flow: LocationRequestFlowHandle,
        completion: LocationCompletion,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(LocationDelegateIvars {
            request_id,
            flow,
            completion,
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn authorization_status_name(status: CLAuthorizationStatus) -> &'static str {
    match status {
        CL_AUTHORIZATION_STATUS_NOT_DETERMINED => "not_determined",
        CL_AUTHORIZATION_STATUS_RESTRICTED => "restricted",
        CL_AUTHORIZATION_STATUS_DENIED => "denied",
        CL_AUTHORIZATION_STATUS_AUTHORIZED_ALWAYS => "authorized_always",
        CL_AUTHORIZATION_STATUS_AUTHORIZED_WHEN_IN_USE => "authorized_when_in_use",
        _ => "unknown",
    }
}

fn is_authorized(status: CLAuthorizationStatus) -> bool {
    matches!(
        status,
        CL_AUTHORIZATION_STATUS_AUTHORIZED_ALWAYS | CL_AUTHORIZATION_STATUS_AUTHORIZED_WHEN_IN_USE
    )
}

fn is_denied_or_restricted(status: CLAuthorizationStatus) -> bool {
    matches!(
        status,
        CL_AUTHORIZATION_STATUS_DENIED | CL_AUTHORIZATION_STATUS_RESTRICTED
    )
}

fn is_bundled_app_runtime() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
        .is_some_and(|path| path.contains(".app/Contents/MacOS/"))
}

unsafe fn clear_delegate(manager: &CLLocationManager) {
    unsafe { manager.setDelegate(None) };
}

fn take_completion_sender(
    completion: &LocationCompletion,
) -> Option<oneshot::Sender<LocationRequestResult>> {
    completion.lock().ok().and_then(|mut sender| sender.take())
}

fn finish_location_request(
    request_id: u64,
    manager: &CLLocationManager,
    completion: &LocationCompletion,
    result: LocationRequestResult,
) {
    unsafe { clear_delegate(manager) };
    ACTIVE_LOCATION_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&request_id);
    });

    if let Some(sender) = take_completion_sender(completion) {
        let _ = sender.send(result);
    }
}

fn cancel_location_request(request_id: u64) {
    ACTIVE_LOCATION_REQUESTS.with(|requests| {
        if let Some(request) = requests.borrow_mut().remove(&request_id) {
            unsafe { clear_delegate(&request.manager) };
        }
    });
}

fn trigger_permission_prompt_if_needed(
    manager: &CLLocationManager,
    flow: &LocationRequestFlowHandle,
    source: &str,
) {
    let should_request = {
        let Ok(mut flow) = flow.lock() else {
            return;
        };
        if flow.prompted_for_authorization {
            false
        } else {
            flow.prompted_for_authorization = true;
            true
        }
    };

    if should_request {
        crate::app_info!(
            "weather",
            "system_locate",
            "CoreLocation authorization not determined, requesting one-shot location to trigger macOS permission prompt ({})",
            source
        );
        manager.requestLocation();
    }
}

fn request_authorized_location_if_needed(
    manager: &CLLocationManager,
    flow: &LocationRequestFlowHandle,
    source: &str,
) {
    let status = CLLocationManager::authorizationStatus();
    let should_request = {
        let Ok(mut flow) = flow.lock() else {
            return;
        };
        if flow.requested_authorized_location {
            false
        } else {
            flow.requested_authorized_location = true;
            true
        }
    };

    if should_request {
        crate::app_info!(
            "weather",
            "system_locate",
            "CoreLocation authorized (status={} {}), requesting one-shot location ({})",
            status,
            authorization_status_name(status),
            source
        );
        manager.requestLocation();
    }
}

fn start_location_request(request_id: u64, completion: LocationCompletion) {
    let initial_status = CLLocationManager::authorizationStatus();

    if is_denied_or_restricted(initial_status) {
        if let Some(sender) = take_completion_sender(&completion) {
            let _ = sender.send(Err(format!(
                "CoreLocation unavailable: status={} {}",
                initial_status,
                authorization_status_name(initial_status)
            )));
        }
        return;
    }

    if !matches!(
        initial_status,
        CL_AUTHORIZATION_STATUS_NOT_DETERMINED
            | CL_AUTHORIZATION_STATUS_AUTHORIZED_ALWAYS
            | CL_AUTHORIZATION_STATUS_AUTHORIZED_WHEN_IN_USE
    ) {
        if let Some(sender) = take_completion_sender(&completion) {
            let _ = sender.send(Err(format!(
                "Unexpected CoreLocation authorization status={} {}",
                initial_status,
                authorization_status_name(initial_status)
            )));
        }
        return;
    }

    let flow = Arc::new(Mutex::new(LocationRequestFlow::default()));
    let delegate = LocationDelegate::new(request_id, Arc::clone(&flow), Arc::clone(&completion));
    let manager = CLLocationManager::new();

    unsafe { manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    manager.setDesiredAccuracy(KCLLOCATION_ACCURACY_KILOMETER);

    ACTIVE_LOCATION_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            request_id,
            ActiveLocationRequest {
                manager: manager.clone(),
                _delegate: delegate.clone(),
            },
        );
    });

    if is_authorized(initial_status) {
        request_authorized_location_if_needed(&manager, &flow, "initial status");
    } else {
        trigger_permission_prompt_if_needed(&manager, &flow, "initial status");
    }
}

pub async fn system_locate(app_handle: tauri::AppHandle) -> Option<(f64, f64)> {
    crate::app_info!(
        "weather",
        "system_locate",
        "Attempting macOS CoreLocation natively"
    );

    #[cfg(debug_assertions)]
    if !is_bundled_app_runtime() {
        crate::app_warn!(
            "weather",
            "system_locate",
            "Skipping CoreLocation in dev runtime because the process is not running from a macOS .app bundle; use a bundled debug/release app to test the system permission prompt"
        );
        return None;
    }

    let request_id = NEXT_LOCATION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    let completion = Arc::new(Mutex::new(Some(tx)));

    if let Err(err) = app_handle.run_on_main_thread({
        let completion = Arc::clone(&completion);
        move || start_location_request(request_id, completion)
    }) {
        crate::app_warn!(
            "weather",
            "system_locate",
            "Failed to dispatch CoreLocation request to main thread: {}",
            err
        );
        return None;
    }

    match tokio::time::timeout(LOCATION_REQUEST_TIMEOUT, rx).await {
        Ok(Ok(Ok((lat, lon)))) => {
            crate::app_info!(
                "weather",
                "system_locate",
                "CoreLocation success: lat={:.4}, lon={:.4}",
                lat,
                lon
            );
            Some((lat, lon))
        }
        Ok(Ok(Err(error))) => {
            crate::app_info!(
                "weather",
                "system_locate",
                "{}",
                crate::truncate_utf8(&error, 300)
            );
            None
        }
        Ok(Err(_)) => {
            crate::app_warn!(
                "weather",
                "system_locate",
                "CoreLocation request ended before returning a result"
            );
            None
        }
        Err(_) => {
            let _ = app_handle.run_on_main_thread(move || cancel_location_request(request_id));
            crate::app_warn!(
                "weather",
                "system_locate",
                "CoreLocation timed out waiting for a callback after {}s",
                LOCATION_REQUEST_TIMEOUT.as_secs()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn authorization_status_helpers_match_expected_values() {
        assert!(super::is_authorized(
            super::CL_AUTHORIZATION_STATUS_AUTHORIZED_ALWAYS
        ));
        assert!(super::is_authorized(
            super::CL_AUTHORIZATION_STATUS_AUTHORIZED_WHEN_IN_USE
        ));
        assert!(!super::is_authorized(
            super::CL_AUTHORIZATION_STATUS_NOT_DETERMINED
        ));
        assert!(super::is_denied_or_restricted(
            super::CL_AUTHORIZATION_STATUS_DENIED
        ));
        assert!(super::is_denied_or_restricted(
            super::CL_AUTHORIZATION_STATUS_RESTRICTED
        ));
        assert!(!super::is_denied_or_restricted(
            super::CL_AUTHORIZATION_STATUS_AUTHORIZED_ALWAYS
        ));
    }
}
