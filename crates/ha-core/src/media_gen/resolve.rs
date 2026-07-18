//! Candidate resolution: which (provider, model) pairs serve a request.
//!
//! Single entry-point (`resolve_candidates`) shared by the chat tools and
//! the design paths so nobody re-implements chain / auto / explicit-model
//! selection. Order of precedence:
//!
//! 1. **Explicit model** (tool `model` argument): `"provider::model"` is an
//!    exact pin; a bare model id must match exactly one usable provider —
//!    a collision errors out asking for the `pid::mid` form. Pinned = no
//!    failover (matches the old `image_generate` behavior).
//! 2. **Configured chain** for the function: primary → fallbacks, dangling
//!    or unusable refs skipped with a warning. A configured chain is
//!    authoritative — exhaustion fails rather than sliding to auto.
//! 3. **Auto**: provider order × capability filter, first matching model
//!    per provider (one candidate per provider keeps failure chains short).

use anyhow::{bail, Result};

use crate::truncate_utf8;

use super::types::{
    ImageModelCaps, MediaFunction, MediaGenConfig, MediaModelConfig, MediaProviderConfig,
};

/// One runnable (provider, model) pair.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedCandidate<'a> {
    pub provider: &'a MediaProviderConfig,
    pub model: &'a MediaModelConfig,
}

impl<'a> ResolvedCandidate<'a> {
    pub fn label(&self) -> String {
        format!("{} ({})", self.model.id, self.provider.name)
    }
}

/// Hint appended to "nothing configured" errors so users land in the right
/// settings surface.
pub const CONFIG_HINT: &str = "Settings → Model Providers → Generation Models";

pub fn resolve_candidates<'a>(
    cfg: &'a MediaGenConfig,
    function: MediaFunction,
    explicit_model: Option<&str>,
) -> Result<Vec<ResolvedCandidate<'a>>> {
    if let Some(spec) = explicit_model
        .map(str::trim)
        .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("auto"))
    {
        return resolve_explicit(cfg, function, spec).map(|c| vec![c]);
    }

    if let Some(chain) = cfg.chains.for_function(function) {
        let mut out = Vec::new();
        for entry in chain.iter() {
            let Some(provider) = cfg.provider(&entry.provider_id) else {
                app_warn!(
                    "media_gen",
                    "resolve",
                    "chain[{function}] references missing provider {}, skipping",
                    entry.provider_id
                );
                continue;
            };
            if !provider.is_usable() {
                app_warn!(
                    "media_gen",
                    "resolve",
                    "chain[{function}] provider {} disabled or missing credentials, skipping",
                    provider.name
                );
                continue;
            }
            let Some(model) = provider.model_config(&entry.model_id) else {
                app_warn!(
                    "media_gen",
                    "resolve",
                    "chain[{function}] references missing model {} on {}, skipping",
                    entry.model_id,
                    provider.name
                );
                continue;
            };
            if !model.serves(function) {
                app_warn!(
                    "media_gen",
                    "resolve",
                    "chain[{function}] model {} on {} cannot serve this function, skipping",
                    model.id,
                    provider.name
                );
                continue;
            }
            out.push(ResolvedCandidate { provider, model });
        }
        if out.is_empty() {
            bail!(
                "the configured default chain for `{function}` has no usable candidates \
                 (providers disabled, deleted, or missing credentials) — fix it in {CONFIG_HINT}"
            );
        }
        return Ok(out);
    }

    // Auto: provider order × every serving model. Multiple models on one
    // provider are all candidates (in declared order) so a request the first
    // model can't satisfy — e.g. n=4 on a max_n=1 model — still reaches a
    // later capable model on the SAME provider before failing over. Capability
    // filtering happens in the executor per candidate; `serves()` only gates
    // modality/kind here, not request geometry.
    let out: Vec<ResolvedCandidate<'a>> = cfg
        .providers
        .iter()
        .filter(|p| p.is_usable())
        .flat_map(|provider| {
            provider
                .models
                .iter()
                .filter(move |m| m.serves(function))
                .map(move |model| ResolvedCandidate { provider, model })
        })
        .collect();
    if out.is_empty() {
        bail!(
            "no media-generation provider configured for `{function}` — add one in {CONFIG_HINT}"
        );
    }
    Ok(out)
}

fn resolve_explicit<'a>(
    cfg: &'a MediaGenConfig,
    function: MediaFunction,
    spec: &str,
) -> Result<ResolvedCandidate<'a>> {
    if let Some((pid, mid)) = spec.split_once("::") {
        let Some(provider) = cfg.provider(pid.trim()) else {
            bail!("unknown media provider id `{}`", truncate_utf8(pid, 120));
        };
        if !provider.is_usable() {
            bail!(
                "media provider {} is disabled or missing credentials",
                provider.name
            );
        }
        let Some(model) = provider.model_config(mid.trim()) else {
            bail!(
                "model `{}` not found on provider {}",
                truncate_utf8(mid, 120),
                provider.name
            );
        };
        if !model.serves(function) {
            bail!(
                "model {} on {} cannot serve `{function}`",
                model.id,
                provider.name
            );
        }
        return Ok(ResolvedCandidate { provider, model });
    }

    // Bare model id: must be unique across usable providers.
    let matches: Vec<ResolvedCandidate<'a>> = cfg
        .providers
        .iter()
        .filter(|p| p.is_usable())
        .filter_map(|provider| {
            provider
                .model_config(spec)
                .filter(|m| m.serves(function))
                .map(|model| ResolvedCandidate { provider, model })
        })
        .collect();
    match matches.len() {
        0 => bail!(
            "model `{}` not found on any usable provider for `{function}` — check {CONFIG_HINT}",
            truncate_utf8(spec, 120)
        ),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => {
            let options: Vec<String> = matches
                .iter()
                .map(|c| format!("{}::{}", c.provider.id, c.model.id))
                .collect();
            bail!(
                "model `{}` exists on multiple providers — disambiguate with the \
                 `provider::model` form: {}",
                truncate_utf8(spec, 120),
                options.join(", ")
            )
        }
    }
}

// ── Request-side capability validation ────────────────────────────

/// What the caller asked for, geometry-wise. Used to skip candidates whose
/// declared caps conflict. A model without a caps group passes everything
/// (lenient — user-added models must not be killed by the gate; the
/// provider rejects what it truly can't do).
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageRequestSpec<'a> {
    /// Explicit non-default size requested?
    pub size: Option<&'a str>,
    pub aspect_ratio: Option<&'a str>,
    pub resolution: Option<&'a str>,
    pub n: u32,
    pub input_images: usize,
    pub has_mask: bool,
}

/// `Ok(())` = candidate acceptable; `Err(reason)` = skip it (reason feeds
/// the failover log).
pub fn validate_image_request(
    caps: Option<&ImageModelCaps>,
    spec: &ImageRequestSpec<'_>,
) -> Result<(), String> {
    let Some(caps) = caps else {
        return Ok(()); // Unknown caps → lenient pass-through.
    };

    if spec.has_mask && !caps.supports_mask {
        return Err("model does not support mask editing".into());
    }

    let edit_mode = spec.input_images > 0 && !spec.has_mask;
    if edit_mode {
        let Some(edit) = &caps.edit else {
            return Err("model does not support image editing".into());
        };
        if spec.input_images > edit.max_input_images as usize {
            return Err(format!(
                "model accepts at most {} input image(s)",
                edit.max_input_images
            ));
        }
        if spec.n > edit.max_n {
            return Err(format!(
                "model produces at most {} image(s) in edit mode",
                edit.max_n
            ));
        }
        if spec.size.is_some() && !edit.supports_size {
            return Err("model does not support custom size in edit mode".into());
        }
        if spec.aspect_ratio.is_some() && !edit.supports_aspect_ratio {
            return Err("model does not support aspect ratio in edit mode".into());
        }
        if spec.resolution.is_some() && !edit.supports_resolution {
            return Err("model does not support resolution in edit mode".into());
        }
    } else {
        if spec.n > caps.max_n {
            return Err(format!("model produces at most {} image(s)", caps.max_n));
        }
        if let Some(size) = spec.size {
            if !caps.supports_size {
                return Err("model does not support custom size".into());
            }
            if !caps.sizes.is_empty() && !caps.sizes.iter().any(|s| s == size) {
                return Err(format!("model does not support size {size}"));
            }
        }
        if let Some(ar) = spec.aspect_ratio {
            if !caps.supports_aspect_ratio {
                return Err("model does not support aspect ratio".into());
            }
            if !caps.aspect_ratios.is_empty() && !caps.aspect_ratios.iter().any(|s| s == ar) {
                return Err(format!("model does not support aspect ratio {ar}"));
            }
        }
        if let Some(res) = spec.resolution {
            if !caps.supports_resolution {
                return Err("model does not support resolution tiers".into());
            }
            if !caps.resolutions.is_empty() && !caps.resolutions.iter().any(|s| s == res) {
                return Err(format!("model does not support resolution {res}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_gen::types::{
        AudioKind, AudioModelCaps, ImageEditCaps, MediaModality, MediaModelChain, MediaModelConfig,
        MediaModelRef, MediaProviderConfig, MediaVendorKind,
    };

    fn image_model(id: &str) -> MediaModelConfig {
        let mut m = MediaModelConfig::new(id, id, MediaModality::Image);
        m.image = Some(ImageModelCaps {
            max_n: 4,
            supports_size: true,
            ..Default::default()
        });
        m
    }

    fn speech_model(id: &str) -> MediaModelConfig {
        let mut m = MediaModelConfig::new(id, id, MediaModality::Audio);
        m.audio = Some(AudioModelCaps {
            kinds: vec![AudioKind::Speech],
            ..Default::default()
        });
        m
    }

    fn usable(name: &str, models: Vec<MediaModelConfig>) -> MediaProviderConfig {
        let mut p = MediaProviderConfig::new(name, MediaVendorKind::Openai);
        p.api_key = "sk".into();
        p.models = models;
        p
    }

    fn chain_ref(pid: &str, mid: &str) -> MediaModelRef {
        MediaModelRef {
            provider_id: pid.into(),
            model_id: mid.into(),
        }
    }

    #[test]
    fn auto_includes_all_serving_models_per_provider_in_order() {
        let mut cfg = MediaGenConfig::default();
        cfg.providers.push(usable(
            "A",
            vec![
                speech_model("tts-a"),
                image_model("img-a"),
                image_model("img-a2"),
            ],
        ));
        cfg.providers.push(usable("B", vec![image_model("img-b")]));
        let mut disabled = usable("C", vec![image_model("img-c")]);
        disabled.enabled = false;
        cfg.providers.push(disabled);

        let candidates = resolve_candidates(&cfg, MediaFunction::Image, None).unwrap();
        let labels: Vec<_> = candidates.iter().map(|c| c.model.id.clone()).collect();
        // Every serving model, in provider then model order; the non-image
        // model and the disabled provider are skipped. Both of A's image
        // models are candidates so a request the first can't satisfy can
        // still reach the second before failing over to B.
        assert_eq!(labels, vec!["img-a", "img-a2", "img-b"]);
    }

    #[test]
    fn auto_errors_when_nothing_matches() {
        let mut cfg = MediaGenConfig::default();
        cfg.providers.push(usable("A", vec![speech_model("tts")]));
        let err = resolve_candidates(&cfg, MediaFunction::Image, None).unwrap_err();
        assert!(err.to_string().contains("no media-generation provider"));
    }

    #[test]
    fn chain_is_authoritative_and_skips_dangling() {
        let mut cfg = MediaGenConfig::default();
        let a = usable("A", vec![image_model("img-a")]);
        let b = usable("B", vec![image_model("img-b")]);
        let (aid, bid) = (a.id.clone(), b.id.clone());
        cfg.providers.push(a);
        cfg.providers.push(b);
        cfg.chains.image = Some(MediaModelChain {
            primary: chain_ref("deleted-provider", "x"),
            fallbacks: vec![chain_ref(&bid, "img-b"), chain_ref(&aid, "missing-model")],
        });

        let candidates = resolve_candidates(&cfg, MediaFunction::Image, None).unwrap();
        let labels: Vec<_> = candidates.iter().map(|c| c.model.id.clone()).collect();
        // Dangling primary + missing model skipped; does NOT slide to auto
        // (img-a is usable but not in the chain).
        assert_eq!(labels, vec!["img-b"]);
    }

    #[test]
    fn exhausted_chain_errors_instead_of_sliding_to_auto() {
        let mut cfg = MediaGenConfig::default();
        cfg.providers.push(usable("A", vec![image_model("img-a")]));
        cfg.chains.image = Some(MediaModelChain {
            primary: chain_ref("gone", "x"),
            fallbacks: vec![],
        });
        let err = resolve_candidates(&cfg, MediaFunction::Image, None).unwrap_err();
        assert!(err.to_string().contains("no usable candidates"));
    }

    #[test]
    fn explicit_pid_mid_pins_single_candidate() {
        let mut cfg = MediaGenConfig::default();
        let a = usable("A", vec![image_model("img")]);
        let aid = a.id.clone();
        cfg.providers.push(a);
        cfg.providers.push(usable("B", vec![image_model("img")]));

        let spec = format!("{aid}::img");
        let candidates =
            resolve_candidates(&cfg, MediaFunction::Image, Some(spec.as_str())).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider.id, aid);
    }

    #[test]
    fn explicit_bare_collision_requires_disambiguation() {
        let mut cfg = MediaGenConfig::default();
        cfg.providers.push(usable("A", vec![image_model("img")]));
        cfg.providers.push(usable("B", vec![image_model("img")]));
        let err = resolve_candidates(&cfg, MediaFunction::Image, Some("img")).unwrap_err();
        assert!(err.to_string().contains("provider::model"));
    }

    #[test]
    fn explicit_bare_unique_match_works_and_auto_keyword_falls_through() {
        let mut cfg = MediaGenConfig::default();
        cfg.providers.push(usable("A", vec![image_model("img-a")]));
        cfg.providers.push(usable("B", vec![image_model("img-b")]));

        let candidates = resolve_candidates(&cfg, MediaFunction::Image, Some("img-b")).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].model.id, "img-b");

        // "auto" / empty behave like no explicit model (all candidates).
        assert_eq!(
            resolve_candidates(&cfg, MediaFunction::Image, Some("auto"))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            resolve_candidates(&cfg, MediaFunction::Image, Some("  "))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn audio_kind_filters_candidates() {
        let mut cfg = MediaGenConfig::default();
        let mut eleven = usable("Eleven", vec![speech_model("eleven_v3")]);
        eleven.models.push({
            let mut m = MediaModelConfig::new("music_v1", "Music", MediaModality::Audio);
            m.audio = Some(AudioModelCaps {
                kinds: vec![AudioKind::Music],
                supports_duration: true,
                ..Default::default()
            });
            m
        });
        cfg.providers.push(eleven);

        let speech =
            resolve_candidates(&cfg, MediaFunction::Audio(AudioKind::Speech), None).unwrap();
        assert_eq!(speech[0].model.id, "eleven_v3");
        let music = resolve_candidates(&cfg, MediaFunction::Audio(AudioKind::Music), None).unwrap();
        assert_eq!(music[0].model.id, "music_v1");
        assert!(resolve_candidates(&cfg, MediaFunction::Audio(AudioKind::Sfx), None).is_err());
    }

    #[test]
    fn validate_image_request_lenient_without_caps_strict_with() {
        let spec = ImageRequestSpec {
            size: Some("999x999"),
            n: 9,
            ..Default::default()
        };
        assert!(validate_image_request(None, &spec).is_ok());

        let caps = ImageModelCaps {
            max_n: 4,
            supports_size: true,
            sizes: vec!["1024x1024".into()],
            ..Default::default()
        };
        assert!(validate_image_request(Some(&caps), &spec).is_err()); // n > max
        let ok_spec = ImageRequestSpec {
            size: Some("1024x1024"),
            n: 2,
            ..Default::default()
        };
        assert!(validate_image_request(Some(&caps), &ok_spec).is_ok());
        let bad_size = ImageRequestSpec {
            size: Some("999x999"),
            n: 1,
            ..Default::default()
        };
        assert!(validate_image_request(Some(&caps), &bad_size).is_err());
    }

    #[test]
    fn validate_mask_and_edit_paths() {
        let caps = ImageModelCaps {
            max_n: 4,
            supports_mask: true,
            edit: None,
            ..Default::default()
        };
        // Mask request on a mask-capable model passes even without edit caps.
        let mask_spec = ImageRequestSpec {
            input_images: 1,
            has_mask: true,
            n: 1,
            ..Default::default()
        };
        assert!(validate_image_request(Some(&caps), &mask_spec).is_ok());
        // Maskless img2img needs edit caps.
        let edit_spec = ImageRequestSpec {
            input_images: 1,
            n: 1,
            ..Default::default()
        };
        assert!(validate_image_request(Some(&caps), &edit_spec).is_err());

        let edit_caps = ImageModelCaps {
            supports_mask: false,
            edit: Some(ImageEditCaps {
                max_n: 2,
                max_input_images: 1,
                supports_size: false,
                supports_aspect_ratio: false,
                supports_resolution: false,
            }),
            ..Default::default()
        };
        assert!(validate_image_request(Some(&edit_caps), &edit_spec).is_ok());
        assert!(validate_image_request(Some(&edit_caps), &mask_spec).is_err()); // no mask support
        let too_many = ImageRequestSpec {
            input_images: 3,
            n: 1,
            ..Default::default()
        };
        assert!(validate_image_request(Some(&edit_caps), &too_many).is_err());
    }
}
