//! Data model for the unified media-generation subsystem (image / audio,
//! `video` reserved).
//!
//! 类型定义已下沉 [`ha_config_schema::media_gen`]，此处原地再导出保持
//! `crate::media_gen::types::*` 路径不变；测试与需要运行时配置的自由函数
//! （[`ssrf_policy_for`]）留在本 crate。

pub use ha_config_schema::media_gen::{
    AudioGenDefaults, AudioKind, AudioModelCaps, ImageEditCaps, ImageGenDefaults, ImageModelCaps,
    MediaDefaultChains, MediaFunction, MediaGenConfig, MediaModality, MediaModelChain,
    MediaModelConfig, MediaModelRef, MediaProviderConfig, MediaVendorKind, AUDIO_DURATIONS_SEC,
    MAX_INPUT_IMAGES, TIMEOUT_CLAMP_SECS, VALID_ASPECT_RATIOS, VALID_RESOLUTIONS,
};

/// `MediaProviderConfig` 的出站 SSRF policy。
///
/// 原为 `MediaProviderConfig` 的固有方法，因需读**运行时全局配置**
/// （`cached_config().ssrf.default_policy`）而移出：该类型已下沉
/// `ha-config-schema`，而 schema 层是纯数据、不得读运行时状态。
/// Rust 要求固有 impl 与类型同 crate，所以这类方法一律留在子系统里做自由函数。
pub fn ssrf_policy_for(provider: &MediaProviderConfig) -> crate::security::ssrf::SsrfPolicy {
    if provider.allow_private_network {
        crate::security::ssrf::SsrfPolicy::AllowPrivate
    } else {
        crate::config::cached_config().ssrf.default_policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_model(id: &str) -> MediaModelConfig {
        let mut m = MediaModelConfig::new(id, id, MediaModality::Image);
        m.image = Some(ImageModelCaps {
            max_n: 4,
            supports_size: true,
            ..Default::default()
        });
        m
    }

    fn audio_model(id: &str, kinds: Vec<AudioKind>) -> MediaModelConfig {
        let mut m = MediaModelConfig::new(id, id, MediaModality::Audio);
        m.audio = Some(AudioModelCaps {
            kinds,
            ..Default::default()
        });
        m
    }

    #[test]
    fn masked_redacts_api_key_and_extra() {
        let mut p = MediaProviderConfig::new("OpenAI", MediaVendorKind::Openai);
        p.api_key = "sk-real-key-1234567890".into();
        p.extra.insert("thinking_level".into(), "HIGH".into());
        let masked = p.masked();
        assert_ne!(masked.api_key, p.api_key);
        assert!(masked.api_key.contains("..."));
        assert_eq!(masked.extra["thinking_level"], "****");
    }

    #[test]
    fn serves_matches_modality_and_kind() {
        let img = image_model("gpt-image-1");
        assert!(img.serves(MediaFunction::Image));
        assert!(!img.serves(MediaFunction::Audio(AudioKind::Speech)));

        let tts = audio_model("eleven_v3", vec![AudioKind::Speech]);
        assert!(tts.serves(MediaFunction::Audio(AudioKind::Speech)));
        assert!(!tts.serves(MediaFunction::Audio(AudioKind::Music)));
        assert!(!tts.serves(MediaFunction::Image));

        // Lenient: no caps group / empty kinds → serves any audio kind.
        let bare = MediaModelConfig::new("custom", "Custom", MediaModality::Audio);
        assert!(bare.serves(MediaFunction::Audio(AudioKind::Music)));
        let empty_kinds = audio_model("x", vec![]);
        assert!(empty_kinds.serves(MediaFunction::Audio(AudioKind::Sfx)));
    }

    #[test]
    fn is_usable_requires_key_except_openai_compatible() {
        let mut p = MediaProviderConfig::new("OpenAI", MediaVendorKind::Openai);
        assert!(!p.is_usable());
        p.api_key = "sk".into();
        assert!(p.is_usable());
        p.enabled = false;
        assert!(!p.is_usable());

        let mut local = MediaProviderConfig::new("Local", MediaVendorKind::OpenaiCompatible);
        assert!(!local.is_usable()); // no base_url yet
        local.base_url = Some("http://127.0.0.1:8080".into());
        assert!(local.is_usable()); // key-less self-hosted is fine
    }

    #[test]
    fn effective_base_url_falls_back_to_vendor_default() {
        let mut p = MediaProviderConfig::new("Fal", MediaVendorKind::Fal);
        assert_eq!(p.effective_base_url(), "https://fal.run");
        p.base_url = Some("  ".into());
        assert_eq!(p.effective_base_url(), "https://fal.run");
        p.base_url = Some("https://proxy.example.com".into());
        assert_eq!(p.effective_base_url(), "https://proxy.example.com");
    }

    #[test]
    fn function_parse_round_trip() {
        for f in [
            MediaFunction::Image,
            MediaFunction::Audio(AudioKind::Speech),
            MediaFunction::Audio(AudioKind::Music),
            MediaFunction::Audio(AudioKind::Sfx),
        ] {
            assert_eq!(MediaFunction::parse(f.as_str()), Some(f));
        }
        assert_eq!(MediaFunction::parse("nope"), None);
    }

    #[test]
    fn timeout_clamped_on_read_not_write() {
        let mut d = ImageGenDefaults {
            timeout_seconds: 5,
            ..Default::default()
        };
        assert_eq!(d.effective_timeout_secs(), 30);
        d.timeout_seconds = 100_000;
        assert_eq!(d.effective_timeout_secs(), 900);
        d.timeout_seconds = 180;
        assert_eq!(d.effective_timeout_secs(), 180);
    }

    #[test]
    fn serde_round_trip_keeps_all_fields() {
        let mut cfg = MediaGenConfig::default();
        let mut p = MediaProviderConfig::new("ElevenLabs", MediaVendorKind::Elevenlabs);
        p.api_key = "xi-test".into();
        p.default_voice = Some("21m00Tcm4TlvDq8ikWAM".into());
        p.models
            .push(audio_model("eleven_v3", vec![AudioKind::Speech]));
        p.models.push({
            let mut m = audio_model("music_v1", vec![AudioKind::Music]);
            m.audio.as_mut().unwrap().supports_duration = true;
            m
        });
        let pid = p.id.clone();
        cfg.providers.push(p);
        cfg.chains.speech = Some(MediaModelChain {
            primary: MediaModelRef {
                provider_id: pid.clone(),
                model_id: "eleven_v3".into(),
            },
            fallbacks: vec![],
        });
        cfg.image_defaults.default_resolution = Some("2K".into());

        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: MediaGenConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].models.len(), 2);
        assert!(
            parsed.providers[0].models[1]
                .audio
                .as_ref()
                .unwrap()
                .supports_duration
        );
        assert_eq!(
            parsed.chains.speech.as_ref().unwrap().primary.model_id,
            "eleven_v3"
        );
        assert_eq!(
            parsed.image_defaults.default_resolution.as_deref(),
            Some("2K")
        );
        assert!(parsed.has_capable_provider(MediaModality::Audio));
        assert!(!parsed.has_capable_provider(MediaModality::Image));
    }

    #[test]
    fn audio_kind_serde_is_lowercase() {
        assert_eq!(
            serde_json::to_string(&AudioKind::Speech).unwrap(),
            "\"speech\""
        );
        let k: AudioKind = serde_json::from_str("\"sfx\"").unwrap();
        assert_eq!(k, AudioKind::Sfx);
    }
}
