//! IM channel tool approval interaction.
//!
//! When a tool requires approval during an IM channel conversation, this module
//! intercepts the `"approval_required"` EventBus event, sends an approval prompt
//! to the IM channel (with buttons if supported, text fallback otherwise), and
//! routes the user's response back to `submit_approval_response()`.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{InlineButton, ReplyPayload};
use crate::tools::approval::{
    submit_approval_response, ApprovalReasonKind, ApprovalReasonPayload, ApprovalResolutionSource,
    ApprovalResponse,
};
use crate::ttl_cache::TtlCache;

use std::sync::Arc;

/// Callback data prefix for approval buttons across all channels.
const APPROVAL_PREFIX: &str = "approval:";

// ── Pending text-reply approvals ─────────────────────────────────

/// Tracks a pending approval that awaits a text reply (for channels without buttons).
#[derive(Debug, Clone)]
struct PendingTextApproval {
    request_id: String,
    forbids_allow_always: bool,
}

/// Registry of pending text-reply approvals, keyed by (account_id, chat_id).
/// Only used for channels that don't support buttons.
static TEXT_PENDING: OnceLock<Mutex<HashMap<(String, String), Vec<PendingTextApproval>>>> =
    OnceLock::new();

fn get_text_pending() -> &'static Mutex<HashMap<(String, String), Vec<PendingTextApproval>>> {
    TEXT_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Throttle for the "you have N pending approvals" hint — one nudge per
/// (account, chat) per the configured interval (see
/// `permission.imApprovalHintThrottleSecs`, default 60s). Backed by
/// [`TtlCache`] so stale entries auto-expire (bounded memory across
/// long-lived IM deployments). Capacity 1024 is generous for any
/// plausible per-process chat count.
static HINT_THROTTLE_CACHE: OnceLock<TtlCache<(String, String), ()>> = OnceLock::new();

fn get_hint_throttle() -> &'static TtlCache<(String, String), ()> {
    HINT_THROTTLE_CACHE.get_or_init(|| TtlCache::new(1024))
}

fn hint_throttle_duration() -> Duration {
    let secs = crate::config::cached_config()
        .permission
        .im_approval_hint_throttle_secs;
    Duration::from_secs(secs)
}

/// Remove any in-memory pending text-reply state for `request_id`. Called by
/// the tool execution path when an approval is timed out / cancelled /
/// otherwise resolved without an IM reply, so stale entries don't
/// accumulate. Mirrors [`super::ask_user::drop_pending_by_request_id`].
pub async fn drop_pending_by_request_id(request_id: &str) {
    let mut map = get_text_pending().lock().await;
    let mut empty_keys = Vec::new();
    for (key, list) in map.iter_mut() {
        list.retain(|p| p.request_id != request_id);
        if list.is_empty() {
            empty_keys.push(key.clone());
        }
    }
    for k in empty_keys {
        map.remove(&k);
    }
}

/// Drop all pending text-reply approval state for a whole session. Called by
/// the session cleanup watcher on delete / purge: resolves the session →
/// (account, chat) IM conversation and clears that chat's `TEXT_PENDING` stack,
/// so a deleted session leaves no stale IM approval entries that could hijack a
/// later reply (SURFACE-2 / INCOG-4). No-op when the session has no attached IM
/// conversation.
pub async fn drop_pending_for_session(session_id: &str) {
    let Some(channel_db) = crate::globals::get_channel_db() else {
        return;
    };
    let conv = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return,
        Err(e) => {
            app_warn!(
                "channel",
                "approval",
                "drop_pending_for_session lookup failed for {}: {}",
                session_id,
                e
            );
            return;
        }
    };
    let key = (conv.account_id, conv.chat_id);
    get_text_pending().lock().await.remove(&key);
}

/// Drop all pending text-reply approval state for a specific (account, chat).
/// Backstop for the IM eviction watcher (G5 / SURFACE-4): after denying each
/// pending approval tool-side, clear the chat's `TEXT_PENDING` stack so a stale
/// text entry can't hijack a later reply in the taken-over chat. Takes the chat
/// coordinates directly (the evicted attach row is already gone from the DB, so
/// `drop_pending_for_session` can't resolve it).
pub async fn drop_pending_for_chat(account_id: &str, chat_id: &str) {
    let key = (account_id.to_string(), chat_id.to_string());
    get_text_pending().lock().await.remove(&key);
}

// ── InlineButton helper ──────────────────────────────────────────

impl InlineButton {
    /// Returns the effective callback identifier: `callback_data` if set, otherwise `text`.
    pub fn callback_id(&self) -> &str {
        self.callback_data.as_deref().unwrap_or(&self.text)
    }
}

// ── Approval button builder ──────────────────────────────────────

/// Build the standard 3-button row for approval prompts.
/// The `callback_data` format is `approval:{request_id}:{action}`.
pub(crate) fn build_approval_buttons(
    request_id: &str,
    reason: Option<&ApprovalReasonPayload>,
) -> Vec<Vec<InlineButton>> {
    let locale = current_locale();
    let mut row = vec![InlineButton {
        text: approval_button_allow_once(locale).to_string(),
        callback_data: Some(format!("{}{}:allow_once", APPROVAL_PREFIX, request_id)),
        url: None,
    }];
    if !reason_forbids_allow_always(reason) {
        row.push(InlineButton {
            text: approval_button_allow_always(locale).to_string(),
            callback_data: Some(format!("{}{}:allow_always", APPROVAL_PREFIX, request_id)),
            url: None,
        });
    }
    row.push(InlineButton {
        text: approval_button_deny(locale).to_string(),
        callback_data: Some(format!("{}{}:deny", APPROVAL_PREFIX, request_id)),
        url: None,
    });
    vec![row]
}

/// Whether the approval reason bars `Allow Always` (strict). Delegates to the
/// canonical [`ApprovalReasonKind::is_strict`] instead of re-listing the strict
/// set here — that keeps the IM AllowAlways gate a single source of truth with
/// the engine/timeout strict set (`is_strict` mirrors `AskReason::
/// forbids_allow_always`, guarded by the `reason_kind_is_strict_matches_ask_reason`
/// drift test), so a future strict reason can never silently slip through on the
/// IM surface.
fn reason_forbids_allow_always(reason: Option<&ApprovalReasonPayload>) -> bool {
    reason.is_some_and(|r| r.kind.is_strict())
}

fn tr(locale: &str, row: [&'static str; 12]) -> &'static str {
    crate::i18n::pick_locale(locale, row)
}

#[cfg(not(test))]
fn current_locale() -> &'static str {
    crate::i18n::current_ui_locale()
}

#[cfg(test)]
fn current_locale() -> &'static str {
    crate::i18n::DEFAULT_LOCALE
}

fn approval_button_allow_once(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "✅ 允许一次",
            "✅ 允許一次",
            "✅ Allow Once",
            "✅ 1回だけ許可",
            "✅ 한 번 허용",
            "✅ Permitir una vez",
            "✅ Permitir uma vez",
            "✅ Разрешить один раз",
            "✅ السماح مرة واحدة",
            "✅ Bir kez izin ver",
            "✅ Cho phép một lần",
            "✅ Benarkan sekali",
        ],
    )
}

fn approval_button_allow_always(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "🔓 总是允许",
            "🔓 總是允許",
            "🔓 Always Allow",
            "🔓 常に許可",
            "🔓 항상 허용",
            "🔓 Permitir siempre",
            "🔓 Permitir sempre",
            "🔓 Всегда разрешать",
            "🔓 السماح دائما",
            "🔓 Her zaman izin ver",
            "🔓 Luôn cho phép",
            "🔓 Sentiasa benarkan",
        ],
    )
}

fn approval_button_deny(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "❌ 拒绝",
            "❌ 拒絕",
            "❌ Deny",
            "❌ 拒否",
            "❌ 거부",
            "❌ Denegar",
            "❌ Negar",
            "❌ Отклонить",
            "❌ رفض",
            "❌ Reddet",
            "❌ Từ chối",
            "❌ Tolak",
        ],
    )
}

/// Render the approval reason as a one-line suffix for IM prompts.
///
/// Protected path details are intentionally redacted: IM approvals can happen
/// in shared chats, and echoing a configured path such as an SSH key location
/// would leak more than the command preview itself.
#[cfg(test)]
fn reason_line(reason: Option<&ApprovalReasonPayload>) -> String {
    reason_line_for_locale(reason, current_locale())
}

fn reason_line_for_locale(reason: Option<&ApprovalReasonPayload>, locale: &str) -> String {
    let Some(r) = reason else {
        return String::new();
    };
    let label = reason_label(r.kind, locale);
    let detail =
        match r.kind {
            ApprovalReasonKind::EditTool => Some(
                tr(
                    locale,
                    [
                        "工具可以修改文件",
                        "工具可以修改檔案",
                        "tool can modify files",
                        "ツールはファイルを変更できます",
                        "도구가 파일을 수정할 수 있습니다",
                        "la herramienta puede modificar archivos",
                        "a ferramenta pode modificar arquivos",
                        "инструмент может изменять файлы",
                        "يمكن للأداة تعديل الملفات",
                        "araç dosyaları değiştirebilir",
                        "công cụ có thể sửa tệp",
                        "alat boleh mengubah fail",
                    ],
                )
                .to_string(),
            ),
            ApprovalReasonKind::EditCommand => prefixed_detail(
                tr(
                    locale,
                    [
                        "匹配编辑命令规则",
                        "符合編輯命令規則",
                        "matched edit-command rule",
                        "編集コマンド規則に一致",
                        "편집 명령 규칙과 일치",
                        "coincide con una regla de comando de edición",
                        "corresponde a uma regra de comando de edição",
                        "совпало с правилом команды редактирования",
                        "طابق قاعدة أمر تعديل",
                        "düzenleme komutu kuralıyla eşleşti",
                        "khớp quy tắc lệnh chỉnh sửa",
                        "sepadan dengan peraturan arahan edit",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::DangerousCommand => prefixed_detail(
                tr(
                    locale,
                    [
                        "匹配危险命令规则",
                        "符合危險命令規則",
                        "matched dangerous-command rule",
                        "危険なコマンド規則に一致",
                        "위험 명령 규칙과 일치",
                        "coincide con una regla de comando peligroso",
                        "corresponde a uma regra de comando perigoso",
                        "совпало с правилом опасной команды",
                        "طابق قاعدة أمر خطير",
                        "tehlikeli komut kuralıyla eşleşti",
                        "khớp quy tắc lệnh nguy hiểm",
                        "sepadan dengan peraturan arahan berbahaya",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::ProtectedPath => Some(
                tr(
                    locale,
                    [
                        "匹配已配置的受保护路径",
                        "符合已設定的受保護路徑",
                        "matched a configured protected path",
                        "設定済みの保護パスに一致",
                        "구성된 보호 경로와 일치",
                        "coincide con una ruta protegida configurada",
                        "corresponde a um caminho protegido configurado",
                        "совпало с настроенным защищенным путем",
                        "طابق مسارا محميا تم تكوينه",
                        "yapılandırılmış korumalı yolla eşleşti",
                        "khớp đường dẫn được bảo vệ đã cấu hình",
                        "sepadan dengan laluan terlindung yang dikonfigurasi",
                    ],
                )
                .to_string(),
            ),
            ApprovalReasonKind::AgentCustomList => Some(
                tr(
                    locale,
                    [
                        "Agent 策略要求此工具先审批",
                        "Agent 策略要求此工具先核准",
                        "agent policy requires approval for this tool",
                        "Agent ポリシーによりこのツールには承認が必要です",
                        "에이전트 정책상 이 도구에는 승인이 필요합니다",
                        "la política del agente requiere aprobación para esta herramienta",
                        "a política do agente exige aprovação para esta ferramenta",
                        "политика агента требует одобрения для этого инструмента",
                        "تتطلب سياسة الوكيل موافقة لهذه الأداة",
                        "aracı politikası bu araç için onay gerektiriyor",
                        "chính sách agent yêu cầu phê duyệt công cụ này",
                        "dasar agen memerlukan kelulusan untuk alat ini",
                    ],
                )
                .to_string(),
            ),
            ApprovalReasonKind::SmartJudge => snippet_detail(&r.detail)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    Some(
                        tr(
                            locale,
                            [
                                "未返回理由；需要请求审批",
                                "未返回理由；需要請求核准",
                                "no rationale returned; asking for approval",
                                "理由が返されていないため承認を求めます",
                                "근거가 반환되지 않아 승인을 요청합니다",
                                "no se devolvió una justificación; se solicita aprobación",
                                "nenhuma justificativa retornada; pedindo aprovação",
                                "обоснование не получено; запрашивается одобрение",
                                "لم يتم إرجاع سبب؛ يتم طلب الموافقة",
                                "gerekçe dönmedi; onay isteniyor",
                                "không có lý do trả về; đang yêu cầu phê duyệt",
                                "tiada rasional dikembalikan; meminta kelulusan",
                            ],
                        )
                        .to_string(),
                    )
                }),
            ApprovalReasonKind::BrowserEvaluate => prefixed_detail(
                tr(
                    locale,
                    [
                        "脚本",
                        "指令碼",
                        "script",
                        "スクリプト",
                        "스크립트",
                        "script",
                        "script",
                        "скрипт",
                        "النص البرمجي",
                        "betik",
                        "tập lệnh",
                        "skrip",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::BrowserRawCdp => prefixed_detail(
                tr(
                    locale,
                    [
                        "CDP 方法",
                        "CDP 方法",
                        "CDP method",
                        "CDP メソッド",
                        "CDP 메서드",
                        "método CDP",
                        "método CDP",
                        "метод CDP",
                        "طريقة CDP",
                        "CDP yöntemi",
                        "phương thức CDP",
                        "kaedah CDP",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::BrowserChromeAccess => prefixed_detail(
                tr(
                    locale,
                    [
                        "Chrome 操作",
                        "Chrome 操作",
                        "Chrome action",
                        "Chrome 操作",
                        "Chrome 작업",
                        "acción de Chrome",
                        "ação do Chrome",
                        "действие Chrome",
                        "إجراء Chrome",
                        "Chrome eylemi",
                        "hành động Chrome",
                        "tindakan Chrome",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::BrowserDownloadAction => prefixed_detail(
                tr(
                    locale,
                    [
                        "下载操作",
                        "下載操作",
                        "download action",
                        "ダウンロード操作",
                        "다운로드 작업",
                        "acción de descarga",
                        "ação de download",
                        "действие загрузки",
                        "إجراء تنزيل",
                        "indirme eylemi",
                        "hành động tải xuống",
                        "tindakan muat turun",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::MacControlAction => prefixed_detail(
                tr(
                    locale,
                    [
                        "操作",
                        "操作",
                        "action",
                        "操作",
                        "작업",
                        "acción",
                        "ação",
                        "действие",
                        "إجراء",
                        "eylem",
                        "hành động",
                        "tindakan",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::MacControlDangerousAction => prefixed_detail(
                tr(
                    locale,
                    [
                        "潜在危险操作",
                        "潛在危險操作",
                        "potentially dangerous action",
                        "危険な可能性のある操作",
                        "잠재적으로 위험한 작업",
                        "acción potencialmente peligrosa",
                        "ação potencialmente perigosa",
                        "потенциально опасное действие",
                        "إجراء قد يكون خطيرا",
                        "olası tehlikeli eylem",
                        "hành động có thể nguy hiểm",
                        "tindakan yang mungkin berbahaya",
                    ],
                ),
                &r.detail,
            ),
            ApprovalReasonKind::PlanModeAsk => Some(
                tr(
                    locale,
                    [
                        "计划模式要求此工具先询问",
                        "計劃模式要求此工具先詢問",
                        "plan mode requires asking before this tool",
                        "計画モードではこのツールの前に確認が必要です",
                        "계획 모드에서는 이 도구 전에 확인이 필요합니다",
                        "el modo Plan requiere preguntar antes de esta herramienta",
                        "o modo Plano exige perguntar antes desta ferramenta",
                        "режим планирования требует спросить перед этим инструментом",
                        "يتطلب وضع الخطة السؤال قبل هذه الأداة",
                        "Plan modu bu araçtan önce sormayı gerektirir",
                        "chế độ lập kế hoạch yêu cầu hỏi trước công cụ này",
                        "mod pelan memerlukan pertanyaan sebelum alat ini",
                    ],
                )
                .to_string(),
            ),
            ApprovalReasonKind::CronDelete => Some(
                tr(
                    locale,
                    [
                        "永久删除计划任务",
                        "永久刪除排程任務",
                        "permanently delete a scheduled task",
                        "スケジュール済みタスクを完全に削除",
                        "예약된 작업을 영구 삭제",
                        "eliminar permanentemente una tarea programada",
                        "excluir permanentemente uma tarefa agendada",
                        "навсегда удалить запланированную задачу",
                        "حذف مهمة مجدولة نهائيا",
                        "zamanlanmış görevi kalıcı olarak sil",
                        "xóa vĩnh viễn tác vụ đã lên lịch",
                        "padam tugasan berjadual secara kekal",
                    ],
                )
                .to_string(),
            ),
        };

    match detail {
        Some(detail) => format!("\n{label}: {detail}"),
        None => format!("\n{label}"),
    }
}

fn reason_label(kind: ApprovalReasonKind, locale: &str) -> &'static str {
    match kind {
        ApprovalReasonKind::EditTool => tr(
            locale,
            [
                "✏ 编辑工具",
                "✏ 編輯工具",
                "✏ Edit Tool",
                "✏ 編集ツール",
                "✏ 편집 도구",
                "✏ Herramienta de edición",
                "✏ Ferramenta de edição",
                "✏ Инструмент редактирования",
                "✏ أداة تحرير",
                "✏ Düzenleme aracı",
                "✏ Công cụ chỉnh sửa",
                "✏ Alat edit",
            ],
        ),
        ApprovalReasonKind::EditCommand => tr(
            locale,
            [
                "✏ 编辑命令",
                "✏ 編輯命令",
                "✏ Edit Command",
                "✏ 編集コマンド",
                "✏ 편집 명령",
                "✏ Comando de edición",
                "✏ Comando de edição",
                "✏ Команда редактирования",
                "✏ أمر تعديل",
                "✏ Düzenleme komutu",
                "✏ Lệnh chỉnh sửa",
                "✏ Arahan edit",
            ],
        ),
        ApprovalReasonKind::DangerousCommand => tr(
            locale,
            [
                "⚠ 危险命令",
                "⚠ 危險命令",
                "⚠ Dangerous Command",
                "⚠ 危険なコマンド",
                "⚠ 위험 명령",
                "⚠ Comando peligroso",
                "⚠ Comando perigoso",
                "⚠ Опасная команда",
                "⚠ أمر خطير",
                "⚠ Tehlikeli komut",
                "⚠ Lệnh nguy hiểm",
                "⚠ Arahan berbahaya",
            ],
        ),
        ApprovalReasonKind::ProtectedPath => tr(
            locale,
            [
                "🛡 受保护路径",
                "🛡 受保護路徑",
                "🛡 Protected Path",
                "🛡 保護されたパス",
                "🛡 보호된 경로",
                "🛡 Ruta protegida",
                "🛡 Caminho protegido",
                "🛡 Защищенный путь",
                "🛡 مسار محمي",
                "🛡 Korumalı yol",
                "🛡 Đường dẫn được bảo vệ",
                "🛡 Laluan terlindung",
            ],
        ),
        ApprovalReasonKind::AgentCustomList => tr(
            locale,
            [
                "⚙ Agent 策略",
                "⚙ Agent 策略",
                "⚙ Agent Policy",
                "⚙ Agent ポリシー",
                "⚙ 에이전트 정책",
                "⚙ Política del agente",
                "⚙ Política do agente",
                "⚙ Политика агента",
                "⚙ سياسة الوكيل",
                "⚙ Aracı politikası",
                "⚙ Chính sách agent",
                "⚙ Dasar agen",
            ],
        ),
        ApprovalReasonKind::SmartJudge => tr(
            locale,
            [
                "💭 智能判断",
                "💭 智慧判斷",
                "💭 Smart Judge",
                "💭 スマート判定",
                "💭 스마트 판단",
                "💭 Juicio inteligente",
                "💭 Julgamento inteligente",
                "💭 Умная оценка",
                "💭 حكم ذكي",
                "💭 Akıllı değerlendirme",
                "💭 Phán đoán thông minh",
                "💭 Pertimbangan pintar",
            ],
        ),
        ApprovalReasonKind::BrowserEvaluate => tr(
            locale,
            [
                "🌐 浏览器 JS",
                "🌐 瀏覽器 JS",
                "🌐 Browser JS",
                "🌐 ブラウザ JS",
                "🌐 브라우저 JS",
                "🌐 JS del navegador",
                "🌐 JS do navegador",
                "🌐 JS браузера",
                "🌐 JavaScript المتصفح",
                "🌐 Tarayıcı JS",
                "🌐 JS trình duyệt",
                "🌐 JS pelayar",
            ],
        ),
        ApprovalReasonKind::BrowserRawCdp => tr(
            locale,
            [
                "⚠ 浏览器 CDP",
                "⚠ 瀏覽器 CDP",
                "⚠ Browser CDP",
                "⚠ ブラウザ CDP",
                "⚠ 브라우저 CDP",
                "⚠ CDP del navegador",
                "⚠ CDP do navegador",
                "⚠ CDP браузера",
                "⚠ CDP المتصفح",
                "⚠ Tarayıcı CDP",
                "⚠ CDP trình duyệt",
                "⚠ CDP pelayar",
            ],
        ),
        ApprovalReasonKind::BrowserChromeAccess => tr(
            locale,
            [
                "🌐 真实 Chrome",
                "🌐 真實 Chrome",
                "🌐 Real Chrome",
                "🌐 実際の Chrome",
                "🌐 실제 Chrome",
                "🌐 Chrome real",
                "🌐 Chrome real",
                "🌐 Реальный Chrome",
                "🌐 Chrome الحقيقي",
                "🌐 Gerçek Chrome",
                "🌐 Chrome thật",
                "🌐 Chrome sebenar",
            ],
        ),
        ApprovalReasonKind::BrowserDownloadAction => tr(
            locale,
            [
                "⚠ 浏览器下载",
                "⚠ 瀏覽器下載",
                "⚠ Browser Download",
                "⚠ ブラウザのダウンロード",
                "⚠ 브라우저 다운로드",
                "⚠ Descarga del navegador",
                "⚠ Download do navegador",
                "⚠ Загрузка браузера",
                "⚠ تنزيل المتصفح",
                "⚠ Tarayıcı indirmesi",
                "⚠ Tải xuống trình duyệt",
                "⚠ Muat turun pelayar",
            ],
        ),
        ApprovalReasonKind::MacControlAction => tr(
            locale,
            [
                "🖥 Mac 控制",
                "🖥 Mac 控制",
                "🖥 Mac Control",
                "🖥 Mac 操作",
                "🖥 Mac 제어",
                "🖥 Control de Mac",
                "🖥 Controle do Mac",
                "🖥 Управление Mac",
                "🖥 تحكم Mac",
                "🖥 Mac kontrolü",
                "🖥 Điều khiển Mac",
                "🖥 Kawalan Mac",
            ],
        ),
        ApprovalReasonKind::MacControlDangerousAction => tr(
            locale,
            [
                "⚠ Mac 控制",
                "⚠ Mac 控制",
                "⚠ Mac Control",
                "⚠ Mac 操作",
                "⚠ Mac 제어",
                "⚠ Control de Mac",
                "⚠ Controle do Mac",
                "⚠ Управление Mac",
                "⚠ تحكم Mac",
                "⚠ Mac kontrolü",
                "⚠ Điều khiển Mac",
                "⚠ Kawalan Mac",
            ],
        ),
        ApprovalReasonKind::PlanModeAsk => tr(
            locale,
            [
                "🧭 计划模式",
                "🧭 計劃模式",
                "🧭 Plan Mode",
                "🧭 計画モード",
                "🧭 계획 모드",
                "🧭 Modo Plan",
                "🧭 Modo Plano",
                "🧭 Режим планирования",
                "🧭 وضع الخطة",
                "🧭 Plan modu",
                "🧭 Chế độ lập kế hoạch",
                "🧭 Mod pelan",
            ],
        ),
        ApprovalReasonKind::CronDelete => tr(
            locale,
            [
                "🗑 删除定时任务",
                "🗑 刪除定時任務",
                "🗑 Cron Delete",
                "🗑 Cron 削除",
                "🗑 Cron 삭제",
                "🗑 Eliminación de cron",
                "🗑 Exclusão de cron",
                "🗑 Удаление cron",
                "🗑 حذف Cron",
                "🗑 Cron silme",
                "🗑 Xóa cron",
                "🗑 Padam cron",
            ],
        ),
    }
}

fn snippet_detail(detail: &Option<String>) -> Option<&str> {
    let trimmed = detail.as_deref()?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(crate::truncate_utf8(trimmed, 280))
}

fn prefixed_detail(prefix: &str, detail: &Option<String>) -> Option<String> {
    let snippet = snippet_detail(detail)?;
    Some(format!("{prefix}: {snippet}"))
}

/// Format the approval prompt text (plain text, no HTML — works across all channels).
fn format_approval_text(command: &str, reason: Option<&ApprovalReasonPayload>) -> String {
    format_approval_text_for_locale(command, reason, current_locale())
}

fn format_approval_text_for_locale(
    command: &str,
    reason: Option<&ApprovalReasonPayload>,
    locale: &str,
) -> String {
    let preview = crate::truncate_utf8(command, 500);
    format!(
        "{}\n\n{}{}",
        approval_required_title(locale),
        preview,
        reason_line_for_locale(reason, locale)
    )
}

/// Short visible tag for a `request_id`, used to disambiguate multiple
/// pending approvals when the user replies. Six UTF-8 chars keeps
/// collisions effectively impossible at the per-(account, chat) scope —
/// `truncate_utf8` stays safe even if the id generator ever moves off
/// ASCII UUIDs.
fn id_tag(request_id: &str) -> &str {
    crate::truncate_utf8(request_id, 6)
}

/// Format the text-only approval prompt (for channels without buttons).
/// Includes the `#tag` so the user can target a specific pending approval
/// (`yes#abc123`) when several are queued; bare replies (`yes` / `1`) fall
/// back to LIFO order.
///
/// `stack_depth` is the number of pending approvals (including this one)
/// in the current (account, chat). When >1 the reply hint nudges the user
/// to disambiguate with `#tag`.
///
/// `timeout_secs` comes from `permission.approval_timeout_secs` so the
/// deadline shown to the user matches the actual timeout. `0` is
/// rendered as "no time limit" — matches the tool-side behaviour where
/// `0` makes the approval wait forever.
fn format_text_approval(
    command: &str,
    reason: Option<&ApprovalReasonPayload>,
    request_id: &str,
    stack_depth: usize,
    timeout_secs: u64,
) -> String {
    format_text_approval_for_locale(
        command,
        reason,
        request_id,
        stack_depth,
        timeout_secs,
        current_locale(),
    )
}

fn format_text_approval_for_locale(
    command: &str,
    reason: Option<&ApprovalReasonPayload>,
    request_id: &str,
    stack_depth: usize,
    timeout_secs: u64,
    locale: &str,
) -> String {
    let preview = crate::truncate_utf8(command, 500);
    let tag = id_tag(request_id);
    let stack_hint = if stack_depth > 1 {
        text_approval_stack_hint(locale, stack_depth, tag)
    } else {
        String::new()
    };
    let reply_header = timeout_reply_header_for_locale(timeout_secs, locale);
    let allow_always_forbidden = reason_forbids_allow_always(reason);
    let always_line = if allow_always_forbidden {
        ""
    } else {
        text_approval_always_line(locale)
    };
    let alias_hint = if allow_always_forbidden {
        text_approval_alias_hint_strict(locale)
    } else {
        text_approval_alias_hint(locale)
    };
    format!(
        "{} #{tag}:\n{preview}{smart}\n\n{reply_header}\n{}{always_line}\n{}\n{alias_hint}{stack_hint}",
        approval_required_title(locale),
        text_approval_allow_once_line(locale),
        text_approval_deny_line(locale),
        smart = reason_line_for_locale(reason, locale)
    )
}

/// Render the "reply within X" header line from a timeout in seconds.
/// `0` → no deadline; whole minutes are formatted as "X min", anything
/// else stays in seconds so weird values like 90 don't get rounded.
fn timeout_reply_header_for_locale(timeout_secs: u64, locale: &str) -> String {
    if timeout_secs == 0 {
        tr(
            locale,
            [
                "请回复（无时间限制）：",
                "請回覆（無時間限制）：",
                "Reply (no time limit):",
                "返信してください（時間制限なし）：",
                "답장해 주세요(시간 제한 없음):",
                "Responde (sin límite de tiempo):",
                "Responda (sem limite de tempo):",
                "Ответьте (без ограничения времени):",
                "رد (بلا حد زمني):",
                "Yanıtlayın (süre sınırı yok):",
                "Trả lời (không giới hạn thời gian):",
                "Balas (tiada had masa):",
            ],
        )
        .to_string()
    } else if timeout_secs % 60 == 0 {
        let mins = timeout_secs / 60;
        let template = tr(
            locale,
            [
                "请在 {mins} 分钟内回复：",
                "請在 {mins} 分鐘內回覆：",
                "Reply within {mins} min:",
                "{mins} 分以内に返信してください：",
                "{mins}분 안에 답장해 주세요:",
                "Responde en {mins} min:",
                "Responda em {mins} min:",
                "Ответьте в течение {mins} мин:",
                "رد خلال {mins} دقيقة:",
                "{mins} dk içinde yanıtlayın:",
                "Trả lời trong {mins} phút:",
                "Balas dalam {mins} min:",
            ],
        );
        template.replace("{mins}", &mins.to_string())
    } else {
        let template = tr(
            locale,
            [
                "请在 {secs} 秒内回复：",
                "請在 {secs} 秒內回覆：",
                "Reply within {secs}s:",
                "{secs} 秒以内に返信してください：",
                "{secs}초 안에 답장해 주세요:",
                "Responde en {secs}s:",
                "Responda em {secs}s:",
                "Ответьте в течение {secs} с:",
                "رد خلال {secs} ثانية:",
                "{secs} sn içinde yanıtlayın:",
                "Trả lời trong {secs} giây:",
                "Balas dalam {secs}s:",
            ],
        );
        template.replace("{secs}", &timeout_secs.to_string())
    }
}

fn approval_required_title(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "🔐 需要工具审批",
            "🔐 需要工具核准",
            "🔐 Tool approval required",
            "🔐 ツールの承認が必要です",
            "🔐 도구 승인이 필요합니다",
            "🔐 Se requiere aprobación de herramienta",
            "🔐 Aprovação de ferramenta necessária",
            "🔐 Требуется одобрение инструмента",
            "🔐 موافقة الأداة مطلوبة",
            "🔐 Araç onayı gerekiyor",
            "🔐 Cần phê duyệt công cụ",
            "🔐 Kelulusan alat diperlukan",
        ],
    )
}

fn text_approval_allow_once_line(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "  1 / yes / ok — 允许一次",
            "  1 / yes / ok — 允許一次",
            "  1 / yes / ok — Allow once",
            "  1 / yes / ok — 1回だけ許可",
            "  1 / yes / ok — 한 번 허용",
            "  1 / yes / ok — Permitir una vez",
            "  1 / yes / ok — Permitir uma vez",
            "  1 / yes / ok — Разрешить один раз",
            "  1 / yes / ok — السماح مرة واحدة",
            "  1 / yes / ok — Bir kez izin ver",
            "  1 / yes / ok — Cho phép một lần",
            "  1 / yes / ok — Benarkan sekali",
        ],
    )
}

fn text_approval_always_line(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "\n  2 / always   — 总是允许",
            "\n  2 / always   — 總是允許",
            "\n  2 / always   — Always allow",
            "\n  2 / always   — 常に許可",
            "\n  2 / always   — 항상 허용",
            "\n  2 / always   — Permitir siempre",
            "\n  2 / always   — Permitir sempre",
            "\n  2 / always   — Всегда разрешать",
            "\n  2 / always   — السماح دائما",
            "\n  2 / always   — Her zaman izin ver",
            "\n  2 / always   — Luôn cho phép",
            "\n  2 / always   — Sentiasa benarkan",
        ],
    )
}

fn text_approval_deny_line(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "  3 / no / deny — 拒绝",
            "  3 / no / deny — 拒絕",
            "  3 / no / deny — Deny",
            "  3 / no / deny — 拒否",
            "  3 / no / deny — 거부",
            "  3 / no / deny — Denegar",
            "  3 / no / deny — Negar",
            "  3 / no / deny — Отклонить",
            "  3 / no / deny — رفض",
            "  3 / no / deny — Reddet",
            "  3 / no / deny — Từ chối",
            "  3 / no / deny — Tolak",
        ],
    )
}

fn text_approval_alias_hint(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "中文也可：同意 / 总是 / 拒绝",
            "中文也可：同意 / 總是 / 拒絕",
            "Chinese aliases also work: 同意 / 总是 / 拒绝",
            "中国語の別名も使えます：同意 / 总是 / 拒绝",
            "중국어 별칭도 사용할 수 있습니다: 同意 / 总是 / 拒绝",
            "También funcionan alias chinos: 同意 / 总是 / 拒绝",
            "Aliases em chinês também funcionam: 同意 / 总是 / 拒绝",
            "Китайские варианты тоже работают: 同意 / 总是 / 拒绝",
            "تعمل الأسماء الصينية أيضا: 同意 / 总是 / 拒绝",
            "Çince takma adlar da çalışır: 同意 / 总是 / 拒绝",
            "Cũng có thể dùng bí danh tiếng Trung: 同意 / 总是 / 拒绝",
            "Alias Cina juga boleh digunakan: 同意 / 总是 / 拒绝",
        ],
    )
}

fn text_approval_alias_hint_strict(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "中文也可：同意 / 拒绝",
            "中文也可：同意 / 拒絕",
            "Chinese aliases also work: 同意 / 拒绝",
            "中国語の別名も使えます：同意 / 拒绝",
            "중국어 별칭도 사용할 수 있습니다: 同意 / 拒绝",
            "También funcionan alias chinos: 同意 / 拒绝",
            "Aliases em chinês também funcionam: 同意 / 拒绝",
            "Китайские варианты тоже работают: 同意 / 拒绝",
            "تعمل الأسماء الصينية أيضا: 同意 / 拒绝",
            "Çince takma adlar da çalışır: 同意 / 拒绝",
            "Cũng có thể dùng bí danh tiếng Trung: 同意 / 拒绝",
            "Alias Cina juga boleh digunakan: 同意 / 拒绝",
        ],
    )
}

fn text_approval_stack_hint(locale: &str, stack_depth: usize, tag: &str) -> String {
    let template = tr(
        locale,
        [
            "\n\n（有 {stack_depth} 个待审批项；追加 `#{tag}` 可指定这一个）",
            "\n\n（有 {stack_depth} 個待核准項；附加 `#{tag}` 可指定這一個）",
            "\n\n({stack_depth} pending — append `#{tag}` to target this one specifically)",
            "\n\n（保留中の承認が {stack_depth} 件あります。`#{tag}` を付けるとこれを指定できます）",
            "\n\n(대기 중인 승인이 {stack_depth}개 있습니다. `#{tag}`를 붙이면 이 항목을 지정합니다)",
            "\n\n({stack_depth} pendientes; añade `#{tag}` para responder a esta en concreto)",
            "\n\n({stack_depth} pendentes; adicione `#{tag}` para responder a esta especificamente)",
            "\n\n({stack_depth} ожидают; добавьте `#{tag}`, чтобы выбрать именно это)",
            "\n\n({stack_depth} معلقة؛ أضف `#{tag}` لاستهداف هذه بالتحديد)",
            "\n\n({stack_depth} bekliyor; özellikle bunu hedeflemek için `#{tag}` ekleyin)",
            "\n\n({stack_depth} mục đang chờ; thêm `#{tag}` để nhắm đúng mục này)",
            "\n\n({stack_depth} menunggu; tambah `#{tag}` untuk menyasarkan yang ini)",
        ],
    );
    template
        .replace("{stack_depth}", &stack_depth.to_string())
        .replace("{tag}", tag)
}

// ── Text reply parsing ───────────────────────────────────────────

/// Parsed approval verb plus an optional `#<id>` suffix the user appended
/// to target a specific pending approval.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReply<'a> {
    response: ApprovalResponse,
    id_suffix: Option<&'a str>,
}

/// Match `raw` against the approval reply whitelist.
///
/// Whitespace-trimmed, case-insensitive, supports both English and Chinese
/// aliases. An optional `#<id>` suffix routes to a specific pending approval
/// instead of the LIFO top (`yes#abc123` / `3#abc123`).
///
/// `AllowAlways` is matched before `AllowOnce` so the literal `yes always`
/// resolves to `AllowAlways` instead of being eaten by the `yes` arm.
fn parse_approval_reply(raw: &str) -> Option<ParsedReply<'_>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (verb_part, id_suffix) = match trimmed.split_once('#') {
        Some((v, id)) => {
            let id_trimmed = id.trim();
            if id_trimmed.is_empty() {
                return None;
            }
            (v.trim_end(), Some(id_trimmed))
        }
        None => (trimmed, None),
    };
    // Whitelist is pure ASCII + CJK; CJK has no case variants and
    // `to_ascii_lowercase` is allocation-free for already-lowercase input,
    // so this is both correct and cheaper than `to_lowercase`. AllowAlways
    // is checked first so `"yes always"` doesn't get eaten by the
    // `"yes"` arm in AllowOnce.
    let lower = verb_part.to_ascii_lowercase();
    let response = if ALLOW_ALWAYS_ALIASES.contains(&lower.as_str()) {
        ApprovalResponse::AllowAlways
    } else if ALLOW_ONCE_ALIASES.contains(&lower.as_str()) {
        ApprovalResponse::AllowOnce
    } else if DENY_ALIASES.contains(&lower.as_str()) {
        ApprovalResponse::Deny
    } else {
        return None;
    };
    Some(ParsedReply {
        response,
        id_suffix,
    })
}

/// `AllowAlways` aliases. Matched **before** [`ALLOW_ONCE_ALIASES`] so
/// `"yes always"` doesn't get eaten by the AllowOnce `"yes"` entry.
/// Adding a language (jp / ko / es / …) is one line per array.
const ALLOW_ALWAYS_ALIASES: &[&str] = &[
    "2",
    "a",
    "always",
    "yes always",
    "yesalways",
    "总是",
    "总是允许",
    "永远",
    "始终",
    "總是",
    "總是允許",
    "永遠",
];

const ALLOW_ONCE_ALIASES: &[&str] = &[
    "1", "y", "yes", "ok", "okay", "allow", "approve", "好", "好的", "同意", "允许", "允許",
    "可以", "行",
];

const DENY_ALIASES: &[&str] = &[
    "3", "n", "no", "deny", "block", "stop", "cancel", "不", "不行", "拒绝", "拒絕", "否", "取消",
];

// ── Shared callback handler (eliminates boilerplate in channel plugins) ──

pub fn spawn_callback_handler_with_source(
    data: &str,
    source: &'static str,
    callback_source: Option<super::ask_user::InteractiveCallbackSource>,
) {
    let data = data.to_string();
    tokio::spawn(async move {
        match handle_approval_callback_with_source(&data, callback_source, source).await {
            Ok(label) => app_info!("channel", source, "Approval: {}", label),
            Err(e) => app_warn!("channel", source, "Approval failed: {}", e),
        }
    });
}

// ── EventBus listener ────────────────────────────────────────────

/// Spawn a background task that listens for `"approval_required"` events on
/// the EventBus and forwards them to the appropriate IM channel.
pub fn spawn_channel_approval_listener(channel_db: Arc<ChannelDB>, registry: Arc<ChannelRegistry>) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app_warn!(
                        "channel",
                        "approval",
                        "Approval listener lagged {} events",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

            match event.name.as_str() {
                "approval_required" => {} // fall through to dispatch below
                "approval_timed_out" => {
                    handle_timeout_event(
                        event.payload.clone(),
                        channel_db.clone(),
                        registry.clone(),
                    )
                    .await;
                    continue;
                }
                // G4 (SURFACE-2): an approval resolved on ANY surface (GUI / HTTP /
                // IM button / timeout / session-delete / eviction) must clear this
                // chat's stale text-reply entry, else it lingers in TEXT_PENDING
                // and a later message gets hijacked as an answer to a dead prompt.
                "approval:resolved" => {
                    if let Some(request_id) =
                        event.payload.get("requestId").and_then(|v| v.as_str())
                    {
                        drop_pending_by_request_id(request_id).await;
                    }
                    continue;
                }
                _ => continue,
            }

            // Deserialize the approval request
            let request: crate::tools::approval::ApprovalRequest =
                match serde_json::from_value(event.payload.clone()) {
                    Ok(r) => r,
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "approval",
                            "Failed to parse approval request: {}",
                            e
                        );
                        continue;
                    }
                };

            let Some(ref session_id) = request.session_id else {
                continue;
            };

            // Look up which channel conversation this session belongs to
            let conversation = match channel_db.get_conversation_by_session(session_id) {
                Ok(Some(conv)) => conv,
                Ok(None) => continue,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "approval",
                        "Failed to look up channel session {}: {}",
                        session_id,
                        e
                    );
                    continue;
                }
            };

            // Load account config
            let store = crate::config::cached_config();
            let account_config = match store.channels.find_account(&conversation.account_id) {
                Some(c) => c.clone(),
                None => continue,
            };

            let channel_id: crate::channel::types::ChannelId = match serde_json::from_value(
                serde_json::Value::String(conversation.channel_id.clone()),
            ) {
                Ok(id) => id,
                Err(_) => continue,
            };

            let supports_buttons = registry
                .get_plugin(&channel_id)
                .map(|p| p.capabilities().supports_buttons)
                .unwrap_or(false);

            // Send the approval prompt to the IM channel
            let payload = if supports_buttons {
                ReplyPayload {
                    text: Some(format_approval_text(
                        &request.command,
                        request.reason.as_ref(),
                    )),
                    buttons: build_approval_buttons(&request.request_id, request.reason.as_ref()),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            } else {
                // Register for text-reply routing. Compute stack_depth inside
                // the same lock so the rendered prompt's "N pending" line
                // matches what `try_handle_approval_reply` will see.
                let key = (
                    conversation.account_id.clone(),
                    conversation.chat_id.clone(),
                );
                let stack_depth = {
                    let mut pending = get_text_pending().lock().await;
                    let list = pending.entry(key).or_default();
                    list.push(PendingTextApproval {
                        request_id: request.request_id.clone(),
                        forbids_allow_always: reason_forbids_allow_always(request.reason.as_ref()),
                    });
                    list.len()
                };

                ReplyPayload {
                    text: Some(format_text_approval(
                        &request.command,
                        request.reason.as_ref(),
                        &request.request_id,
                        stack_depth,
                        crate::tools::approval::approval_timeout_secs(),
                    )),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            };

            if let Err(e) = registry
                .send_reply(&account_config, &conversation.chat_id, &payload)
                .await
            {
                app_warn!(
                    "channel",
                    "approval",
                    "Failed to send approval prompt to channel: {}",
                    e
                );
            }
        }
    });
}

/// Wire payload of the `approval_timed_out` EventBus event. Tools side
/// (`tools::approval::check_and_request_approval` timeout branch) emits
/// this so the IM channel listener can notify the user — the actual list
/// cleanup is independently handled by [`drop_pending_by_request_id`]
/// also called from the tools side, so this listener never has to touch
/// `TEXT_PENDING`.
#[derive(serde::Deserialize)]
struct ApprovalTimedOut {
    request_id: String,
    session_id: Option<String>,
    #[serde(default)]
    timeout_secs: u64,
    /// What the tool path did after the timeout. Determines whether the
    /// IM notification says "denied" (the tool call was blocked) or
    /// "continued anyway" (the tool ran with no human approval per
    /// `permission.approval_timeout_action=proceed`). Optional only for
    /// forward-compat with payloads emitted before the field existed;
    /// missing → assume default `Deny`.
    #[serde(default)]
    timeout_action: crate::config::ApprovalTimeoutAction,
}

/// Tell the IM user the approval prompt expired. Best-effort — if the
/// channel is offline we just log; the tool-side timeout (deny / proceed
/// per config) has already taken effect, and the IM-side `TEXT_PENDING`
/// entry has already been cleared by the tools side calling
/// [`drop_pending_by_request_id`].
async fn handle_timeout_event(
    payload: serde_json::Value,
    channel_db: Arc<ChannelDB>,
    registry: Arc<ChannelRegistry>,
) {
    let event: ApprovalTimedOut = match serde_json::from_value(payload) {
        Ok(e) => e,
        Err(err) => {
            app_warn!(
                "channel",
                "approval",
                "Failed to parse approval_timed_out payload: {}",
                err
            );
            return;
        }
    };
    let Some(session_id) = event.session_id else {
        return;
    };

    let conversation = match channel_db.get_conversation_by_session(&session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return, // not an IM session — desktop handles its own UI
        Err(e) => {
            app_warn!(
                "channel",
                "approval",
                "Timeout lookup failed for session {}: {}",
                session_id,
                e
            );
            return;
        }
    };

    let store = crate::config::cached_config();
    let locale = crate::i18n::effective_ui_locale(&store);
    let account_config = match store.channels.find_account(&conversation.account_id) {
        Some(c) => c.clone(),
        None => return,
    };

    let tag = id_tag(&event.request_id);
    let timeout_secs = event.timeout_secs;
    let body = approval_timeout_notice(locale, tag, timeout_secs, event.timeout_action);
    let payload = ReplyPayload {
        text: Some(body),
        thread_id: conversation.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &conversation.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send approval-timeout notice: {}",
            e
        );
    }
}

fn approval_timeout_notice(
    locale: &str,
    tag: &str,
    timeout_secs: u64,
    action: crate::config::ApprovalTimeoutAction,
) -> String {
    let template = match action {
        crate::config::ApprovalTimeoutAction::Deny => tr(
            locale,
            [
                "⏱ 工具审批 `#{tag}` 已在 {secs} 秒后超时。工具调用已被拒绝；如果仍要执行，请再问我一次。",
                "⏱ 工具核准 `#{tag}` 已在 {secs} 秒後逾時。工具呼叫已被拒絕；如果仍要執行，請再問我一次。",
                "⏱ Tool approval #{tag} timed out after {secs}s. The tool call has been denied — ask me again if you still want it to run.",
                "⏱ ツール承認 `#{tag}` は {secs} 秒後にタイムアウトしました。ツール呼び出しは拒否されました。まだ実行したい場合はもう一度依頼してください。",
                "⏱ 도구 승인 `#{tag}`가 {secs}초 후 시간 초과되었습니다. 도구 호출은 거부되었습니다. 여전히 실행하려면 다시 요청해 주세요.",
                "⏱ La aprobación de herramienta `#{tag}` agotó el tiempo tras {secs}s. La llamada se denegó; vuelve a pedírmelo si aún quieres ejecutarla.",
                "⏱ A aprovação da ferramenta `#{tag}` expirou após {secs}s. A chamada foi negada; peça novamente se ainda quiser executá-la.",
                "⏱ Одобрение инструмента `#{tag}` истекло через {secs} с. Вызов инструмента отклонен; попросите снова, если все еще хотите выполнить его.",
                "⏱ انتهت مهلة موافقة الأداة `#{tag}` بعد {secs} ثانية. تم رفض استدعاء الأداة؛ اسألني مرة أخرى إذا كنت لا تزال تريد تشغيلها.",
                "⏱ Araç onayı `#{tag}` {secs} sn sonra zaman aşımına uğradı. Araç çağrısı reddedildi; hâlâ çalıştırmak istiyorsanız tekrar isteyin.",
                "⏱ Phê duyệt công cụ `#{tag}` đã hết hạn sau {secs} giây. Lệnh gọi công cụ đã bị từ chối; hãy hỏi lại nếu bạn vẫn muốn chạy.",
                "⏱ Kelulusan alat `#{tag}` tamat masa selepas {secs}s. Panggilan alat telah ditolak; minta saya lagi jika masih mahu menjalankannya.",
            ],
        ),
        // `proceed` means the tool path didn't block: it ran the tool
        // anyway. Tell the user clearly so they don't assume the action
        // was cancelled — side effects already happened.
        crate::config::ApprovalTimeoutAction::Proceed => tr(
            locale,
            [
                "⏱ 工具审批 `#{tag}` 已在 {secs} 秒后超时。工具调用已按 `permission.approval_timeout_action=proceed` 继续执行；任何副作用都已经发生。",
                "⏱ 工具核准 `#{tag}` 已在 {secs} 秒後逾時。工具呼叫已依 `permission.approval_timeout_action=proceed` 繼續執行；任何副作用都已發生。",
                "⏱ Tool approval #{tag} timed out after {secs}s. The tool call continued anyway (per `permission.approval_timeout_action=proceed`) — any side effects have already happened.",
                "⏱ ツール承認 `#{tag}` は {secs} 秒後にタイムアウトしました。`permission.approval_timeout_action=proceed` によりツール呼び出しは続行されました。副作用はすでに発生しています。",
                "⏱ 도구 승인 `#{tag}`가 {secs}초 후 시간 초과되었습니다. `permission.approval_timeout_action=proceed`에 따라 도구 호출은 계속되었습니다. 부작용은 이미 발생했습니다.",
                "⏱ La aprobación de herramienta `#{tag}` agotó el tiempo tras {secs}s. La llamada continuó de todos modos (por `permission.approval_timeout_action=proceed`); cualquier efecto secundario ya ocurrió.",
                "⏱ A aprovação da ferramenta `#{tag}` expirou após {secs}s. A chamada continuou mesmo assim (por `permission.approval_timeout_action=proceed`); quaisquer efeitos colaterais já ocorreram.",
                "⏱ Одобрение инструмента `#{tag}` истекло через {secs} с. Вызов все равно продолжился (по `permission.approval_timeout_action=proceed`); побочные эффекты уже произошли.",
                "⏱ انتهت مهلة موافقة الأداة `#{tag}` بعد {secs} ثانية. استمر استدعاء الأداة رغم ذلك (حسب `permission.approval_timeout_action=proceed`)؛ أي آثار جانبية حدثت بالفعل.",
                "⏱ Araç onayı `#{tag}` {secs} sn sonra zaman aşımına uğradı. Araç çağrısı yine de devam etti (`permission.approval_timeout_action=proceed`); yan etkiler zaten gerçekleşti.",
                "⏱ Phê duyệt công cụ `#{tag}` đã hết hạn sau {secs} giây. Lệnh gọi công cụ vẫn tiếp tục (theo `permission.approval_timeout_action=proceed`); mọi tác dụng phụ đã xảy ra.",
                "⏱ Kelulusan alat `#{tag}` tamat masa selepas {secs}s. Panggilan alat tetap diteruskan (mengikut `permission.approval_timeout_action=proceed`); sebarang kesan sampingan sudah berlaku.",
            ],
        ),
    };
    template
        .replace("{tag}", tag)
        .replace("{secs}", &timeout_secs.to_string())
}

// ── Text-reply approval handler ──────────────────────────────────

/// Try to handle an inbound message as an approval text reply.
///
/// Returns `true` if the message was consumed as an approval reply,
/// `false` if it should proceed through normal message processing.
pub async fn try_handle_approval_reply(msg: &crate::channel::types::MsgContext) -> bool {
    let Some(raw) = msg.text.as_deref() else {
        return false;
    };
    let Some(parsed) = parse_approval_reply(raw) else {
        return false;
    };

    let key = (msg.account_id.clone(), msg.chat_id.clone());
    // Snapshot the available tags before popping so we can build a
    // helpful "did you mean" reply when the suffix doesn't match.
    enum TextReplySelection {
        Popped(PendingTextApproval),
        Missing { available_tags: Vec<String> },
        AlwaysUnavailable { tag: String },
    }

    let selection = {
        let mut pending = get_text_pending().lock().await;
        let Some(list) = pending.get_mut(&key) else {
            return false;
        };
        if list.is_empty() {
            pending.remove(&key);
            return false;
        }
        // `#<id>` suffix targets a specific pending approval by short tag
        // (`id_tag` prefix match). Without suffix, fall back to LIFO so the
        // most-recently-prompted approval is the default — matches what's
        // visually on screen.
        let maybe_idx = match parsed.id_suffix {
            Some(target) => list
                .iter()
                .position(|entry| id_tag(&entry.request_id) == target)
                .map(Some)
                .unwrap_or(None),
            None => Some(list.len() - 1),
        };
        match maybe_idx {
            Some(idx)
                if parsed.response == ApprovalResponse::AllowAlways
                    && list[idx].forbids_allow_always =>
            {
                TextReplySelection::AlwaysUnavailable {
                    tag: id_tag(&list[idx].request_id).to_string(),
                }
            }
            Some(idx) => {
                let popped = list.remove(idx);
                if list.is_empty() {
                    pending.remove(&key);
                }
                TextReplySelection::Popped(popped)
            }
            None => {
                let available_tags: Vec<String> = list
                    .iter()
                    .map(|entry| id_tag(&entry.request_id).to_string())
                    .collect();
                TextReplySelection::Missing { available_tags }
            }
        }
    };

    let entry = match selection {
        TextReplySelection::Popped(entry) => entry,
        TextReplySelection::AlwaysUnavailable { tag } => {
            send_allow_always_unavailable_notice(msg, &tag).await;
            return true;
        }
        TextReplySelection::Missing { available_tags } => {
            // Suffix typo: the user clearly tried to reply to an approval
            // (verb parsed, `#<tag>` provided) but the tag doesn't match any
            // pending entry. Consume the message and tell them which tags are
            // valid — falling through to a fresh chat turn would silently
            // route the typo to the LLM and leave the approval pending.
            if let Some(target) = parsed.id_suffix {
                send_suffix_mismatch_notice(msg, target, &available_tags).await;
                return true;
            }
            return false;
        }
    };
    let request_id = entry.request_id;

    // G3 (SURFACE-3): mirror the button path's session<->chat check. The
    // TEXT_PENDING entry is keyed by (account, chat), but the pending approval's
    // session may have been re-attached to a DIFFERENT chat since the prompt was
    // sent (1:1 handover/takeover). Verify the replying conversation still owns
    // the approval's session before submitting; on mismatch, notify + consume
    // (don't leak the reply to the LLM, don't resolve from the wrong chat).
    match crate::tools::approval::pending_approval_session_id(&request_id).await {
        Ok(Some(session_id)) => {
            let reply_source = super::ask_user::InteractiveCallbackSource::new(
                msg.channel_id.clone(),
                msg.account_id.clone(),
                msg.chat_id.clone(),
                msg.thread_id.as_deref(),
            );
            if let Err(e) = super::ask_user::validate_callback_source_for_session(
                &session_id,
                Some(&reply_source),
                "text_reply",
            ) {
                app_warn!(
                    "channel",
                    "approval",
                    "Text approval reply source mismatch for {}: {}",
                    request_id,
                    e
                );
                send_source_mismatch_notice(msg, &request_id).await;
                return true;
            }
        }
        // No session id recorded — can't validate. Fall through to submit;
        // submit_approval_response itself returns NotPending if it's already gone.
        Ok(None) => {}
        Err(e) => {
            app_warn!(
                "channel",
                "approval",
                "Text approval reply session lookup failed for {}: {}",
                request_id,
                e
            );
            send_source_mismatch_notice(msg, &request_id).await;
            return true;
        }
    }

    match submit_approval_response(&request_id, parsed.response, ApprovalResolutionSource::Im).await
    {
        Ok(()) => true,
        Err(e) => {
            // Approval already expired (5-min timeout) — don't consume the message
            app_warn!(
                "channel",
                "approval",
                "Approval expired or invalid ({}), passing message through",
                e
            );
            false
        }
    }
}

/// Tell the user their `#<tag>` suffix didn't match any pending approval.
/// Lists the tags that ARE pending so the typo is fixable in one message.
async fn send_suffix_mismatch_notice(
    msg: &crate::channel::types::MsgContext,
    target: &str,
    available_tags: &[String],
) {
    let store = crate::config::cached_config();
    let locale = crate::i18n::effective_ui_locale(&store);
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };
    let body = suffix_mismatch_notice(locale, target, available_tags);
    let registry = match crate::globals::get_channel_registry() {
        Some(r) => r,
        None => return,
    };
    let payload = ReplyPayload {
        text: Some(body),
        thread_id: msg.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &msg.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send suffix-mismatch notice: {}",
            e
        );
    }
}

/// G3 (SURFACE-3): tell the user their text reply came from a different
/// conversation than the one the approval was sent to (the approval's session
/// has since been attached elsewhere). The approval is left pending; the user
/// must answer it from the chat where the prompt currently lives.
async fn send_source_mismatch_notice(msg: &crate::channel::types::MsgContext, request_id: &str) {
    let store = crate::config::cached_config();
    let locale = crate::i18n::effective_ui_locale(&store);
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };
    let registry = match crate::globals::get_channel_registry() {
        Some(r) => r,
        None => return,
    };
    let tag = id_tag(request_id);
    let payload = ReplyPayload {
        text: Some(source_mismatch_notice(locale, tag)),
        thread_id: msg.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &msg.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send source-mismatch notice: {}",
            e
        );
    }
}

/// Tell text-reply users that this strict approval cannot be persisted.
/// Leave the approval pending so they can still reply `1` / `yes` or deny it.
async fn send_allow_always_unavailable_notice(msg: &crate::channel::types::MsgContext, tag: &str) {
    let store = crate::config::cached_config();
    let locale = crate::i18n::effective_ui_locale(&store);
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };
    let registry = match crate::globals::get_channel_registry() {
        Some(r) => r,
        None => return,
    };
    let payload = ReplyPayload {
        text: Some(allow_always_unavailable_notice(locale, tag)),
        thread_id: msg.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &msg.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send AllowAlways-unavailable notice: {}",
            e
        );
    }
}

/// Best-effort nudge for users whose chat has pending text-mode approvals
/// but who sent something that isn't a reply (e.g. a fresh question while
/// the approval prompt is still up). Sends one line per (account, chat)
/// per [`hint_throttle_duration`] (configurable), not on every non-
/// matching message.
///
/// No-op for accounts on button-capable channels (they never have entries
/// in `TEXT_PENDING`). Called by the dispatcher after
/// [`try_handle_approval_reply`] returns `false`.
pub async fn maybe_send_pending_hint(
    msg: &crate::channel::types::MsgContext,
    registry: &ChannelRegistry,
) {
    let key = (msg.account_id.clone(), msg.chat_id.clone());

    let stack_depth = {
        let pending = get_text_pending().lock().await;
        pending.get(&key).map(|list| list.len()).unwrap_or(0)
    };
    if stack_depth == 0 {
        return;
    }

    // Throttle gate: skip if we already nudged this chat recently. The
    // `TtlCache` bounds memory (capacity 1024) so long-running IM
    // deployments don't accumulate one entry per ever-seen (account, chat).
    let throttle = get_hint_throttle();
    if throttle.get(&key, hint_throttle_duration()).is_some() {
        return;
    }
    throttle.put(key, ());

    let store = crate::config::cached_config();
    let locale = crate::i18n::effective_ui_locale(&store);
    let Some(account_config) = store.channels.find_account(&msg.account_id).cloned() else {
        return;
    };

    let body = pending_approval_hint(locale, stack_depth);
    let payload = ReplyPayload {
        text: Some(body),
        thread_id: msg.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &msg.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "approval",
            "Failed to send pending-approval hint: {}",
            e
        );
    }
}

fn suffix_mismatch_notice(locale: &str, target: &str, available_tags: &[String]) -> String {
    if available_tags.is_empty() {
        // Race: pending was popped between parse and our reply. Don't
        // surface a misleading "available tags: <none>" string.
        let template = tr(
            locale,
            [
                "ℹ️ 标签 `#{target}` 不匹配任何待审批项（可能刚刚已回答或已超时）。",
                "ℹ️ 標籤 `#{target}` 不符合任何待核准項（可能剛剛已回答或已逾時）。",
                "ℹ️ Tag `#{target}` doesn't match any pending approval (it may have just been answered or timed out).",
                "ℹ️ タグ `#{target}` は保留中の承認と一致しません（すでに回答済みかタイムアウトした可能性があります）。",
                "ℹ️ 태그 `#{target}`가 대기 중인 승인과 일치하지 않습니다(방금 응답되었거나 시간 초과되었을 수 있습니다).",
                "ℹ️ La etiqueta `#{target}` no coincide con ninguna aprobación pendiente (puede que ya se haya respondido o vencido).",
                "ℹ️ A tag `#{target}` não corresponde a nenhuma aprovação pendente (talvez já tenha sido respondida ou expirado).",
                "ℹ️ Тег `#{target}` не соответствует ожидающему одобрению (возможно, оно уже отвечено или истекло).",
                "ℹ️ الوسم `#{target}` لا يطابق أي موافقة معلقة (ربما تمت الإجابة عنها أو انتهت مهلتها).",
                "ℹ️ `#{target}` etiketi bekleyen hiçbir onayla eşleşmiyor (az önce yanıtlanmış veya zaman aşımına uğramış olabilir).",
                "ℹ️ Thẻ `#{target}` không khớp phê duyệt nào đang chờ (có thể vừa được trả lời hoặc đã hết hạn).",
                "ℹ️ Tag `#{target}` tidak sepadan dengan sebarang kelulusan menunggu (mungkin baru dijawab atau tamat masa).",
            ],
        );
        return template.replace("{target}", target);
    }

    let tag_list = available_tags
        .iter()
        .map(|t| format!("`#{t}`"))
        .collect::<Vec<_>>()
        .join(" / ");
    let first = &available_tags[0];
    let template = tr(
        locale,
        [
            "ℹ️ 标签 `#{target}` 不匹配任何待审批项。当前待审批：{tag_list}。例如回复 `yes#{first}` 或 `no#{first}`。",
            "ℹ️ 標籤 `#{target}` 不符合任何待核准項。目前待核准：{tag_list}。例如回覆 `yes#{first}` 或 `no#{first}`。",
            "ℹ️ Tag `#{target}` doesn't match any pending approval. Currently pending: {tag_list}. Reply e.g. `yes#{first}` or `no#{first}`.",
            "ℹ️ タグ `#{target}` は保留中の承認と一致しません。現在の保留: {tag_list}。例: `yes#{first}` または `no#{first}` と返信してください。",
            "ℹ️ 태그 `#{target}`가 대기 중인 승인과 일치하지 않습니다. 현재 대기: {tag_list}. 예: `yes#{first}` 또는 `no#{first}`로 답장하세요.",
            "ℹ️ La etiqueta `#{target}` no coincide con ninguna aprobación pendiente. Pendientes: {tag_list}. Responde, por ejemplo, `yes#{first}` o `no#{first}`.",
            "ℹ️ A tag `#{target}` não corresponde a nenhuma aprovação pendente. Pendentes: {tag_list}. Responda, por exemplo, `yes#{first}` ou `no#{first}`.",
            "ℹ️ Тег `#{target}` не соответствует ожидающему одобрению. Сейчас ожидают: {tag_list}. Ответьте, например, `yes#{first}` или `no#{first}`.",
            "ℹ️ الوسم `#{target}` لا يطابق أي موافقة معلقة. المعلق حاليا: {tag_list}. رد مثلا `yes#{first}` أو `no#{first}`.",
            "ℹ️ `#{target}` etiketi bekleyen hiçbir onayla eşleşmiyor. Bekleyenler: {tag_list}. Örn. `yes#{first}` veya `no#{first}` yanıtlayın.",
            "ℹ️ Thẻ `#{target}` không khớp phê duyệt nào đang chờ. Đang chờ: {tag_list}. Ví dụ trả lời `yes#{first}` hoặc `no#{first}`.",
            "ℹ️ Tag `#{target}` tidak sepadan dengan sebarang kelulusan menunggu. Sedang menunggu: {tag_list}. Balas cth. `yes#{first}` atau `no#{first}`.",
        ],
    );
    template
        .replace("{tag_list}", &tag_list)
        .replace("{first}", first)
        .replace("{target}", target)
}

fn source_mismatch_notice(locale: &str, tag: &str) -> String {
    let template = tr(
        locale,
        [
            "ℹ️ 审批 `#{tag}` 现在属于另一个会话，不能从这里回答。请在当前显示该审批提示的聊天里回复。",
            "ℹ️ 核准 `#{tag}` 目前屬於另一個對話，不能從這裡回答。請在目前顯示該核准提示的聊天中回覆。",
            "ℹ️ Approval `#{tag}` belongs to a different conversation now and can't be answered from here. Reply in the chat where the approval prompt currently appears.",
            "ℹ️ 承認 `#{tag}` は現在別の会話に属しているため、ここからは回答できません。承認プロンプトが表示されているチャットで返信してください。",
            "ℹ️ 승인 `#{tag}`는 이제 다른 대화에 속해 있어 여기서 응답할 수 없습니다. 승인 프롬프트가 표시된 채팅에서 답장해 주세요.",
            "ℹ️ La aprobación `#{tag}` ahora pertenece a otra conversación y no puede responderse desde aquí. Responde en el chat donde aparece el aviso de aprobación.",
            "ℹ️ A aprovação `#{tag}` agora pertence a outra conversa e não pode ser respondida daqui. Responda no chat onde o prompt de aprovação aparece.",
            "ℹ️ Одобрение `#{tag}` теперь относится к другому разговору, и здесь на него нельзя ответить. Ответьте в чате, где сейчас показан запрос одобрения.",
            "ℹ️ الموافقة `#{tag}` تنتمي الآن إلى محادثة أخرى ولا يمكن الرد عليها من هنا. رد في الدردشة التي يظهر فيها طلب الموافقة حاليا.",
            "ℹ️ `#{tag}` onayı artık farklı bir konuşmaya ait ve buradan yanıtlanamaz. Onay isteminin göründüğü sohbette yanıtlayın.",
            "ℹ️ Phê duyệt `#{tag}` hiện thuộc một cuộc trò chuyện khác và không thể trả lời từ đây. Hãy trả lời trong cuộc trò chuyện đang hiển thị lời nhắc phê duyệt.",
            "ℹ️ Kelulusan `#{tag}` kini milik perbualan lain dan tidak boleh dijawab dari sini. Balas dalam sembang tempat gesaan kelulusan sedang dipaparkan.",
        ],
    );
    template.replace("{tag}", tag)
}

fn allow_always_unavailable_notice(locale: &str, tag: &str) -> String {
    let template = tr(
        locale,
        [
            "ℹ️ 审批 `#{tag}` 必须逐次确认。回复 `1` / `yes` 允许一次，或回复 `3` / `no` 拒绝。",
            "ℹ️ 核准 `#{tag}` 必須逐次確認。回覆 `1` / `yes` 允許一次，或回覆 `3` / `no` 拒絕。",
            "ℹ️ Approval `#{tag}` requires per-call confirmation. Reply `1` / `yes` to allow once, or `3` / `no` to deny.",
            "ℹ️ 承認 `#{tag}` は毎回の確認が必要です。`1` / `yes` で1回だけ許可、`3` / `no` で拒否します。",
            "ℹ️ 승인 `#{tag}`는 호출마다 확인이 필요합니다. 한 번 허용하려면 `1` / `yes`, 거부하려면 `3` / `no`로 답장하세요.",
            "ℹ️ La aprobación `#{tag}` requiere confirmación por llamada. Responde `1` / `yes` para permitir una vez, o `3` / `no` para denegar.",
            "ℹ️ A aprovação `#{tag}` exige confirmação a cada chamada. Responda `1` / `yes` para permitir uma vez, ou `3` / `no` para negar.",
            "ℹ️ Одобрение `#{tag}` требует подтверждения для каждого вызова. Ответьте `1` / `yes`, чтобы разрешить один раз, или `3` / `no`, чтобы отклонить.",
            "ℹ️ الموافقة `#{tag}` تتطلب تأكيدا لكل استدعاء. رد `1` / `yes` للسماح مرة واحدة، أو `3` / `no` للرفض.",
            "ℹ️ `#{tag}` onayı her çağrı için ayrı onay gerektirir. Bir kez izin vermek için `1` / `yes`, reddetmek için `3` / `no` yanıtlayın.",
            "ℹ️ Phê duyệt `#{tag}` cần xác nhận từng lần gọi. Trả lời `1` / `yes` để cho phép một lần, hoặc `3` / `no` để từ chối.",
            "ℹ️ Kelulusan `#{tag}` memerlukan pengesahan setiap panggilan. Balas `1` / `yes` untuk benarkan sekali, atau `3` / `no` untuk tolak.",
        ],
    );
    template.replace("{tag}", tag)
}

fn pending_approval_hint(locale: &str, stack_depth: usize) -> String {
    let template = tr(
        locale,
        [
            "ℹ️ 你有 {stack_depth} 个待处理的工具审批。我会把这条当作新消息处理。回复 `1` / `yes` 允许，回复 `3` / `no` 拒绝；也可以追加 `#<tag>` 指定某一个。",
            "ℹ️ 你有 {stack_depth} 個待處理的工具核准。我會把這則當作新訊息處理。回覆 `1` / `yes` 允許，回覆 `3` / `no` 拒絕；也可以附加 `#<tag>` 指定某一個。",
            "ℹ️ You have {stack_depth} pending tool approval(s). Treating this as a new message. Reply `1` / `yes` to allow, `3` / `no` to deny — or append `#<tag>` to target a specific one.",
            "ℹ️ 保留中のツール承認が {stack_depth} 件あります。このメッセージは新しいメッセージとして扱います。`1` / `yes` で許可、`3` / `no` で拒否、または `#<tag>` を付けて対象を指定できます。",
            "ℹ️ 대기 중인 도구 승인이 {stack_depth}개 있습니다. 이 메시지는 새 메시지로 처리합니다. `1` / `yes`로 허용, `3` / `no`로 거부하거나 `#<tag>`를 붙여 특정 항목을 지정하세요.",
            "ℹ️ Tienes {stack_depth} aprobación(es) de herramienta pendientes. Trataré esto como un mensaje nuevo. Responde `1` / `yes` para permitir, `3` / `no` para denegar, o añade `#<tag>` para elegir una específica.",
            "ℹ️ Você tem {stack_depth} aprovação(ões) de ferramenta pendente(s). Vou tratar isto como uma nova mensagem. Responda `1` / `yes` para permitir, `3` / `no` para negar, ou adicione `#<tag>` para escolher uma específica.",
            "ℹ️ У вас {stack_depth} ожидающих одобрений инструментов. Считаю это новым сообщением. Ответьте `1` / `yes`, чтобы разрешить, `3` / `no`, чтобы отклонить, или добавьте `#<tag>` для выбора конкретного.",
            "ℹ️ لديك {stack_depth} موافقات أدوات معلقة. سأتعامل مع هذا كرسالة جديدة. رد `1` / `yes` للسماح، أو `3` / `no` للرفض، أو أضف `#<tag>` لاستهداف واحدة محددة.",
            "ℹ️ Bekleyen {stack_depth} araç onayınız var. Bunu yeni mesaj olarak ele alıyorum. İzin vermek için `1` / `yes`, reddetmek için `3` / `no` yanıtlayın veya belirli birini hedeflemek için `#<tag>` ekleyin.",
            "ℹ️ Bạn có {stack_depth} phê duyệt công cụ đang chờ. Tôi sẽ coi đây là tin nhắn mới. Trả lời `1` / `yes` để cho phép, `3` / `no` để từ chối, hoặc thêm `#<tag>` để nhắm một mục cụ thể.",
            "ℹ️ Anda mempunyai {stack_depth} kelulusan alat menunggu. Saya akan menganggap ini sebagai mesej baharu. Balas `1` / `yes` untuk benarkan, `3` / `no` untuk tolak, atau tambah `#<tag>` untuk sasarkan yang khusus.",
        ],
    );
    template.replace("{stack_depth}", &stack_depth.to_string())
}

fn approval_callback_allowed_once(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "✅ 已允许一次",
            "✅ 已允許一次",
            "✅ Allowed (once)",
            "✅ 1回だけ許可しました",
            "✅ 한 번 허용됨",
            "✅ Permitido una vez",
            "✅ Permitido uma vez",
            "✅ Разрешено один раз",
            "✅ تم السماح مرة واحدة",
            "✅ Bir kez izin verildi",
            "✅ Đã cho phép một lần",
            "✅ Dibenarkan sekali",
        ],
    )
}

fn approval_callback_allowed_always(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "🔓 已总是允许",
            "🔓 已總是允許",
            "🔓 Always allowed",
            "🔓 常に許可しました",
            "🔓 항상 허용됨",
            "🔓 Permitido siempre",
            "🔓 Permitido sempre",
            "🔓 Всегда разрешено",
            "🔓 تم السماح دائما",
            "🔓 Her zaman izin verildi",
            "🔓 Đã luôn cho phép",
            "🔓 Sentiasa dibenarkan",
        ],
    )
}

fn approval_callback_denied(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "❌ 已拒绝",
            "❌ 已拒絕",
            "❌ Denied",
            "❌ 拒否しました",
            "❌ 거부됨",
            "❌ Denegado",
            "❌ Negado",
            "❌ Отклонено",
            "❌ تم الرفض",
            "❌ Reddedildi",
            "❌ Đã từ chối",
            "❌ Ditolak",
        ],
    )
}

// ── Callback approval handler (for button-based channels) ────────

pub async fn handle_approval_callback_with_source(
    callback_data: &str,
    callback_source: Option<super::ask_user::InteractiveCallbackSource>,
    source: &'static str,
) -> anyhow::Result<String> {
    let rest = callback_data
        .strip_prefix(APPROVAL_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an approval callback"))?;

    let (request_id, action) = rest
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("Invalid approval callback format"))?;

    let locale = current_locale();
    let (response, label) = match action {
        "allow_once" => (
            ApprovalResponse::AllowOnce,
            approval_callback_allowed_once(locale),
        ),
        "allow_always" => (
            ApprovalResponse::AllowAlways,
            approval_callback_allowed_always(locale),
        ),
        "deny" => (ApprovalResponse::Deny, approval_callback_denied(locale)),
        _ => return Err(anyhow::anyhow!("Unknown approval action: {}", action)),
    };

    // G2 (MISC-11): an approval callback MUST carry a verifiable source so we can
    // confirm it came from the chat that received the prompt — otherwise a click
    // from a different conversation could resolve someone else's approval.
    // Fail-closed: look up the session and validate ALWAYS; a missing source
    // (`None`) can't be validated, so refuse. Safe for approvals — we just sent
    // the prompt message, so a real button click always carries it (Telegram's
    // no-message-callback edge only affects >48h-old / inline buttons, never a
    // live 5-min approval prompt). The shared `validate_callback_source_for_session`
    // keeps its permissive `None → Ok` for the *ask_user* path (Telegram
    // no-message Q&A answers are lower-risk and out of MISC-11 scope); approvals
    // gate here instead so that change can't regress ask_user.
    let session_id = crate::tools::approval::pending_approval_session_id(request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Pending approval {} has no session id", request_id))?;
    let Some(source_ref) = callback_source.as_ref() else {
        return Err(anyhow::anyhow!(
            "Approval callback from {} has no source to validate against session {}; refusing (MISC-11 fail-closed)",
            source,
            session_id
        ));
    };
    super::ask_user::validate_callback_source_for_session(&session_id, Some(source_ref), source)?;

    submit_approval_response(request_id, response, ApprovalResolutionSource::Im).await?;
    Ok(label.to_string())
}

/// Check if a callback data string is an approval callback.
pub fn is_approval_callback(data: &str) -> bool {
    data.starts_with(APPROVAL_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smart(detail: Option<&str>) -> ApprovalReasonPayload {
        reason(ApprovalReasonKind::SmartJudge, detail)
    }

    fn reason(kind: ApprovalReasonKind, detail: Option<&str>) -> ApprovalReasonPayload {
        ApprovalReasonPayload {
            kind,
            detail: detail.map(|s| s.to_string()),
        }
    }

    fn other_reason() -> ApprovalReasonPayload {
        reason(ApprovalReasonKind::DangerousCommand, Some("rm -rf"))
    }

    #[test]
    fn reason_line_renders_smart_judge() {
        assert_eq!(reason_line(None), "");
        assert_eq!(
            reason_line(Some(&smart(None))),
            "\n💭 Smart Judge: no rationale returned; asking for approval"
        );
        assert_eq!(
            reason_line(Some(&smart(Some("   ")))),
            "\n💭 Smart Judge: no rationale returned; asking for approval"
        );
        let line = reason_line(Some(&smart(Some("looks risky"))));
        assert_eq!(line, "\n💭 Smart Judge: looks risky");
    }

    #[test]
    fn reason_line_renders_all_known_reason_kinds() {
        let cases = [
            (
                ApprovalReasonKind::EditTool,
                None,
                "\n✏ Edit Tool: tool can modify files",
            ),
            (
                ApprovalReasonKind::EditCommand,
                Some("apply_patch"),
                "\n✏ Edit Command: matched edit-command rule: apply_patch",
            ),
            (
                ApprovalReasonKind::DangerousCommand,
                Some("rm -rf"),
                "\n⚠ Dangerous Command: matched dangerous-command rule: rm -rf",
            ),
            (
                ApprovalReasonKind::AgentCustomList,
                None,
                "\n⚙ Agent Policy: agent policy requires approval for this tool",
            ),
            (
                ApprovalReasonKind::BrowserEvaluate,
                Some("document.title"),
                "\n🌐 Browser JS: script: document.title",
            ),
            (
                ApprovalReasonKind::BrowserRawCdp,
                Some("Accessibility.getFullAXTree"),
                "\n⚠ Browser CDP: CDP method: Accessibility.getFullAXTree",
            ),
            (
                ApprovalReasonKind::BrowserDownloadAction,
                Some("cancel download 7"),
                "\n⚠ Browser Download: download action: cancel download 7",
            ),
            (
                ApprovalReasonKind::MacControlAction,
                Some("click"),
                "\n🖥 Mac Control: action: click",
            ),
            (
                ApprovalReasonKind::MacControlDangerousAction,
                Some("delete file"),
                "\n⚠ Mac Control: potentially dangerous action: delete file",
            ),
            (
                ApprovalReasonKind::PlanModeAsk,
                None,
                "\n🧭 Plan Mode: plan mode requires asking before this tool",
            ),
        ];

        for (kind, detail, expected) in cases {
            assert_eq!(
                reason_line(Some(&reason(kind, detail))),
                expected,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn reason_line_redacts_protected_path_detail() {
        let line = reason_line(Some(&reason(
            ApprovalReasonKind::ProtectedPath,
            Some("/Users/alice/.ssh/id_rsa"),
        )));
        assert_eq!(
            line,
            "\n🛡 Protected Path: matched a configured protected path"
        );
        assert!(!line.contains(".ssh"));
        assert!(!line.contains("id_rsa"));
    }

    #[test]
    fn reason_line_truncates_long_detail() {
        let long = "x".repeat(1000);
        let line = reason_line(Some(&smart(Some(&long))));
        assert!(line.starts_with("\n💭 Smart Judge: "));
        assert!(line.len() <= 320, "got len {}", line.len());
    }

    #[test]
    fn format_approval_text_includes_reason_line_for_smart_judge() {
        let txt = format_approval_text("exec ls", Some(&smart(Some("trusted dir"))));
        assert!(txt.starts_with("🔐 Tool approval required\n\nexec ls"));
        assert!(txt.contains("💭 Smart Judge: trusted dir"));
    }

    #[test]
    fn format_approval_text_omits_line_when_no_reason() {
        assert!(!format_approval_text("exec ls", None).contains("Smart Judge"));
        assert!(
            format_approval_text("exec ls", Some(&other_reason())).contains("Dangerous Command")
        );
    }

    #[test]
    fn format_text_approval_keeps_numeric_reply_block() {
        let txt = format_text_approval(
            "exec ls",
            Some(&smart(Some("ok per project rules"))),
            "abc123def456",
            1,
            300,
        );
        assert!(txt.contains("💭 Smart Judge: ok per project rules"));
        assert!(txt.contains("1 / yes / ok"));
        assert!(txt.contains("3 / no / deny"));
        // The visible #tag uses the 6-char prefix of the request id.
        assert!(txt.contains("#abc123"));
        // Smart Judge must precede the digit list so 1/2/3 parsing isn't shifted.
        let smart_idx = txt.find("Smart Judge").expect("has smart line");
        let reply_idx = txt.find("Reply within").expect("has reply block");
        assert!(smart_idx < reply_idx);
    }

    #[test]
    fn format_text_approval_hides_always_for_strict_reason() {
        let txt = format_text_approval(
            "exec rm -rf /",
            Some(&ApprovalReasonPayload {
                kind: ApprovalReasonKind::DangerousCommand,
                detail: Some("rm -rf".to_string()),
            }),
            "abc123def456",
            1,
            300,
        );
        assert!(txt.contains("1 / yes / ok"));
        assert!(!txt.contains("2 / always"));
        assert!(!txt.contains("总是"));
        assert!(txt.contains("3 / no / deny"));
    }

    #[test]
    fn format_text_approval_renders_stack_hint_when_multiple_pending() {
        let single = format_text_approval("exec ls", None, "abcdef123456", 1, 300);
        assert!(!single.contains("pending"));

        let multi = format_text_approval("exec ls", None, "abcdef123456", 3, 300);
        assert!(multi.contains("3 pending"));
        assert!(multi.contains("#abcdef"));
    }

    #[test]
    fn format_text_approval_renders_configured_timeout() {
        // Default 5-minute timeout reads as "Reply within 5 min".
        let default = format_text_approval("exec ls", None, "abcdef123456", 1, 300);
        assert!(default.contains("Reply within 5 min:"));

        // Custom whole-minute timeout follows the same shape.
        let two_min = format_text_approval("exec ls", None, "abcdef123456", 1, 120);
        assert!(two_min.contains("Reply within 2 min:"));

        // Non-whole-minute timeout stays in seconds — no rounding.
        let ninety = format_text_approval("exec ls", None, "abcdef123456", 1, 90);
        assert!(ninety.contains("Reply within 90s:"));

        // `0` = no time limit; the deadline phrase changes so the user
        // doesn't assume a 5-min cutoff that doesn't exist.
        let unlimited = format_text_approval("exec ls", None, "abcdef123456", 1, 0);
        assert!(unlimited.contains("Reply (no time limit):"));
        assert!(!unlimited.contains("Reply within"));
    }

    #[test]
    fn parse_approval_reply_accepts_english_aliases() {
        for (input, expected) in [
            ("1", ApprovalResponse::AllowOnce),
            ("yes", ApprovalResponse::AllowOnce),
            ("YES", ApprovalResponse::AllowOnce),
            ("  Yes  ", ApprovalResponse::AllowOnce),
            ("y", ApprovalResponse::AllowOnce),
            ("ok", ApprovalResponse::AllowOnce),
            ("allow", ApprovalResponse::AllowOnce),
            ("2", ApprovalResponse::AllowAlways),
            ("always", ApprovalResponse::AllowAlways),
            ("yes always", ApprovalResponse::AllowAlways),
            ("3", ApprovalResponse::Deny),
            ("no", ApprovalResponse::Deny),
            ("N", ApprovalResponse::Deny),
            ("deny", ApprovalResponse::Deny),
            ("cancel", ApprovalResponse::Deny),
        ] {
            let parsed = parse_approval_reply(input).unwrap_or_else(|| panic!("failed: {input:?}"));
            assert_eq!(parsed.response, expected, "input {input:?}");
            assert!(parsed.id_suffix.is_none(), "input {input:?}");
        }
    }

    #[test]
    fn parse_approval_reply_accepts_chinese_aliases() {
        for (input, expected) in [
            ("好", ApprovalResponse::AllowOnce),
            ("好的", ApprovalResponse::AllowOnce),
            ("同意", ApprovalResponse::AllowOnce),
            ("允许", ApprovalResponse::AllowOnce),
            ("允許", ApprovalResponse::AllowOnce),
            ("总是", ApprovalResponse::AllowAlways),
            ("總是", ApprovalResponse::AllowAlways),
            ("總是允許", ApprovalResponse::AllowAlways),
            ("永远", ApprovalResponse::AllowAlways),
            ("永遠", ApprovalResponse::AllowAlways),
            ("不", ApprovalResponse::Deny),
            ("拒绝", ApprovalResponse::Deny),
            ("拒絕", ApprovalResponse::Deny),
            ("取消", ApprovalResponse::Deny),
        ] {
            let parsed = parse_approval_reply(input).unwrap_or_else(|| panic!("failed: {input:?}"));
            assert_eq!(parsed.response, expected, "input {input:?}");
        }
    }

    #[test]
    fn parse_approval_reply_rejects_unrelated_text() {
        // Avoid false positives — "yesterday" must not match "yes" via
        // prefix or contains.
        assert!(parse_approval_reply("yesterday").is_none());
        assert!(parse_approval_reply("notnow").is_none());
        assert!(parse_approval_reply("好像").is_none());
        assert!(parse_approval_reply("").is_none());
        assert!(parse_approval_reply("   ").is_none());
        assert!(parse_approval_reply("帮我看看天气").is_none());
        assert!(parse_approval_reply("yes please").is_none());
    }

    #[test]
    fn parse_approval_reply_extracts_id_suffix() {
        let parsed = parse_approval_reply("yes#abc123").unwrap();
        assert_eq!(parsed.response, ApprovalResponse::AllowOnce);
        assert_eq!(parsed.id_suffix, Some("abc123"));

        let parsed = parse_approval_reply("3#xyz789").unwrap();
        assert_eq!(parsed.response, ApprovalResponse::Deny);
        assert_eq!(parsed.id_suffix, Some("xyz789"));

        // Trim whitespace around the suffix too.
        let parsed = parse_approval_reply("同意 #abc123 ").unwrap();
        assert_eq!(parsed.response, ApprovalResponse::AllowOnce);
        assert_eq!(parsed.id_suffix, Some("abc123"));

        // Empty suffix is rejected — `yes#` would otherwise route nowhere.
        assert!(parse_approval_reply("yes#").is_none());
        assert!(parse_approval_reply("yes#   ").is_none());
    }

    // The two tests below pin the **list-manipulation primitives** the
    // dispatcher path relies on (LIFO `pop` for bare verbs, `position +
    // remove` for `#tag` suffix). They deliberately do NOT call
    // `try_handle_approval_reply` end-to-end because that requires a live
    // `tools::approval::PENDING_APPROVALS` entry (which would need a
    // `pub(crate)` test hook into a private struct).

    #[tokio::test]
    async fn text_pending_list_pop_is_lifo_for_bare_verb() {
        let key = ("acct-lifo".to_string(), "chat-lifo".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "older-id-aaa".to_string(),
                    forbids_allow_always: false,
                });
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "newer-id-bbb".to_string(),
                    forbids_allow_always: false,
                });
        }
        // Bare "yes" parses with no suffix → dispatcher uses `list.pop()`.
        let parsed = parse_approval_reply("yes").unwrap();
        assert!(parsed.id_suffix.is_none());

        let popped = {
            let mut pending = get_text_pending().lock().await;
            let list = pending.get_mut(&key).unwrap();
            let entry = list.pop().unwrap();
            if list.is_empty() {
                pending.remove(&key);
            }
            entry
        };
        assert_eq!(popped.request_id, "newer-id-bbb");

        let mut pending = get_text_pending().lock().await;
        pending.remove(&key);
    }

    #[tokio::test]
    async fn text_pending_id_tag_position_match_routes_to_non_top() {
        let key = ("acct-suffix".to_string(), "chat-suffix".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "aaaaaa-older".to_string(),
                    forbids_allow_always: false,
                });
            pending
                .entry(key.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "bbbbbb-newer".to_string(),
                    forbids_allow_always: false,
                });
        }

        let parsed = parse_approval_reply("yes#aaaaaa").unwrap();
        assert_eq!(parsed.id_suffix, Some("aaaaaa"));

        // Dispatcher path on `#tag` reply: `iter().position(|e|
        // id_tag(&e.request_id) == target).map(|i| list.remove(i))`.
        let popped = {
            let mut pending = get_text_pending().lock().await;
            let list = pending.get_mut(&key).unwrap();
            let idx = list
                .iter()
                .position(|e| id_tag(&e.request_id) == "aaaaaa")
                .expect("targeted entry exists");
            let entry = list.remove(idx);
            if list.is_empty() {
                pending.remove(&key);
            }
            entry
        };
        assert_eq!(popped.request_id, "aaaaaa-older");
        let pending = get_text_pending().lock().await;
        let remaining = pending.get(&key).expect("newer entry still queued");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].request_id, "bbbbbb-newer");
        drop(pending);

        let mut pending = get_text_pending().lock().await;
        pending.remove(&key);
    }

    #[tokio::test]
    async fn drop_pending_by_request_id_clears_across_chats() {
        let key_a = ("acct-drop".to_string(), "chat-a".to_string());
        let key_b = ("acct-drop".to_string(), "chat-b".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(key_a.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "shared-id-xyz".to_string(),
                    forbids_allow_always: false,
                });
            pending
                .entry(key_b.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "shared-id-xyz".to_string(),
                    forbids_allow_always: false,
                });
            pending
                .entry(key_b.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "unrelated-id-pdq".to_string(),
                    forbids_allow_always: false,
                });
        }

        drop_pending_by_request_id("shared-id-xyz").await;

        let pending = get_text_pending().lock().await;
        assert!(
            pending.get(&key_a).is_none(),
            "chat A entry should be cleared and the now-empty list removed",
        );
        let remaining_b = pending
            .get(&key_b)
            .expect("chat B still has unrelated entry");
        assert_eq!(remaining_b.len(), 1);
        assert_eq!(remaining_b[0].request_id, "unrelated-id-pdq");
        drop(pending);

        let mut pending = get_text_pending().lock().await;
        pending.remove(&key_b);
    }

    #[tokio::test]
    async fn drop_pending_for_chat_clears_only_target_chat() {
        // G5 (SURFACE-4): eviction clears the taken-over chat's text stack only.
        let evicted = ("acct-evict".to_string(), "chat-evicted".to_string());
        let other = ("acct-evict".to_string(), "chat-other".to_string());
        {
            let mut pending = get_text_pending().lock().await;
            pending
                .entry(evicted.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "evicted-req".to_string(),
                    forbids_allow_always: false,
                });
            pending
                .entry(other.clone())
                .or_default()
                .push(PendingTextApproval {
                    request_id: "other-req".to_string(),
                    forbids_allow_always: false,
                });
        }

        drop_pending_for_chat("acct-evict", "chat-evicted").await;

        let mut pending = get_text_pending().lock().await;
        assert!(
            pending.get(&evicted).is_none(),
            "evicted chat's text stack should be cleared",
        );
        assert!(
            pending.get(&other).is_some(),
            "other chat must be untouched",
        );
        pending.remove(&other);
    }

    #[test]
    fn id_tag_clamps_to_six_chars_or_shorter() {
        assert_eq!(id_tag("abcdef123456"), "abcdef");
        assert_eq!(id_tag("ab"), "ab");
        assert_eq!(id_tag(""), "");
    }
}
