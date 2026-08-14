//! Thin CLI adapter for the shared pet import pipeline and desktop activation API.
//!
//! `preview` deliberately destroys its in-process preview token before the
//! command exits. A later `import` re-fetches/re-reads the source and requires
//! the package hash the user reviewed; if the source changed, installation
//! fails closed instead of committing different bytes.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{
    cancel_import_preview, commit_import, list_pets, preview_import_async, PetConfig,
    PetImportCommitRequest, PetImportPreview, PetImportPreviewRequest, PetImportSource,
    PetManifest, PetRef, PetSummary, PetValidationIssue,
};

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Capabilities {
        json: bool,
    },
    Activate(ActivateOptions),
    List {
        json: bool,
    },
    Preview(ImportOptions),
    Import {
        options: ImportOptions,
        expected_package_hash: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
struct ImportOptions {
    sources: Vec<String>,
    display_name: Option<String>,
    json: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct ActivateOptions {
    pet_ref: PetRef,
    json: bool,
}

const MAX_CLI_SOURCES: usize = 64;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesOutput {
    status: &'static str,
    schema_version: u8,
    hope_agent_version: &'static str,
    source_kinds: [&'static str; 5],
    repeated_local_source: bool,
    expected_package_hash: bool,
    activate_installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewOutput {
    status: &'static str,
    manifest: PetManifest,
    width: u32,
    height: u32,
    issues: Vec<PetValidationIssue>,
    asset_hash: String,
    package_hash: String,
    duplicate_pet_ref: Option<PetRef>,
    can_commit: bool,
}

impl From<&PetImportPreview> for PreviewOutput {
    fn from(preview: &PetImportPreview) -> Self {
        Self {
            status: "preview",
            manifest: preview.manifest.clone(),
            width: preview.width,
            height: preview.height,
            issues: preview.issues.clone(),
            asset_hash: preview.asset_hash.clone(),
            package_hash: preview.package_hash.clone(),
            duplicate_pet_ref: preview.duplicate_pet_ref.clone(),
            can_commit: preview.can_commit(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportOutput {
    status: &'static str,
    pet: PetSummary,
    imported: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivateOutput {
    status: &'static str,
    pet_ref: PetRef,
    enabled: bool,
}

/// Run `hope-agent pet ...` after the shell has wired feature crates and
/// initialized the data directories. The adapter does not initialize the
/// chat runtime or open sessions.db.
pub async fn run(args: &[String]) -> Result<()> {
    match parse_command(args)? {
        Command::Help => print_help(),
        Command::Capabilities { json } => print_capabilities(json)?,
        Command::Activate(options) => activate(options).await?,
        Command::List { json } => {
            let snapshot = ha_core::blocking::run_blocking(list_pets).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                for pet in snapshot.pets {
                    println!("{}\t{}", pet.pet_ref.0, pet.manifest.display_name);
                }
            }
        }
        Command::Preview(options) => preview(options).await?,
        Command::Import {
            options,
            expected_package_hash,
        } => import(options, &expected_package_hash).await?,
    }
    Ok(())
}

async fn activate(options: ActivateOptions) -> Result<()> {
    let bind_addr = ha_core::config::cached_config().server.bind_addr.clone();
    let endpoint = desktop_pet_activate_url(&bind_addr)?;
    let token = ha_core::blocking::run_blocking(ha_core::server_auth::load_managed_token).await?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut request = client.post(endpoint).json(&serde_json::json!({
        "petRef": options.pet_ref.clone(),
    }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .context("pet_cli_desktop_activate_unavailable")?;
    if !response.status().is_success() {
        anyhow::bail!(
            "pet_cli_desktop_activate_failed:{}",
            response.status().as_u16()
        );
    }
    let config: PetConfig = response
        .json()
        .await
        .context("pet_cli_desktop_activate_response_invalid")?;
    if !config.enabled || config.selected_pet_ref != options.pet_ref {
        anyhow::bail!("pet_cli_desktop_activate_response_mismatch");
    }
    let output = ActivateOutput {
        status: "activated",
        pet_ref: config.selected_pet_ref,
        enabled: config.enabled,
    };
    if options.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Activated: {}", output.pet_ref.0);
    }
    Ok(())
}

fn desktop_pet_activate_url(bind_addr: &str) -> Result<url::Url> {
    let addr: SocketAddr = bind_addr
        .trim()
        .parse()
        .context("pet_cli_desktop_bind_invalid")?;
    let ip = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ip if ip.is_loopback() => ip,
        _ => anyhow::bail!("pet_cli_desktop_bind_not_loopback"),
    };
    url::Url::parse(&format!(
        "http://{}/api/pets/activate",
        SocketAddr::new(ip, addr.port())
    ))
    .context("pet_cli_desktop_endpoint_invalid")
}

async fn preview(options: ImportOptions) -> Result<()> {
    let preview = preview_import_async(request_from_options(&options)?).await?;
    let output = PreviewOutput::from(&preview);
    cancel_import_preview(preview.preview_token)
        .await
        .context("pet_cli_preview_cleanup_failed")?;
    print_preview(output, options.json)
}

async fn import(options: ImportOptions, expected_package_hash: &str) -> Result<()> {
    validate_package_hash(expected_package_hash)?;
    let preview = preview_import_async(request_from_options(&options)?).await?;
    if preview.package_hash != expected_package_hash {
        let current = preview.package_hash.clone();
        cancel_import_preview(preview.preview_token)
            .await
            .context("pet_cli_preview_cleanup_failed")?;
        anyhow::bail!(
            "pet_cli_source_changed: expected {expected_package_hash}, current {current}; preview again before importing"
        );
    }
    if !preview.can_commit() {
        let error_codes = preview
            .issues
            .iter()
            .filter(|issue| issue.severity == crate::PetValidationSeverity::Error)
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>()
            .join(",");
        cancel_import_preview(preview.preview_token)
            .await
            .context("pet_cli_preview_cleanup_failed")?;
        anyhow::bail!("pet_preview_has_errors:{error_codes}");
    }
    let result = commit_import(PetImportCommitRequest {
        preview_token: preview.preview_token,
        enable_after_import: false,
    })
    .await?;
    let output = ImportOutput {
        status: if result.imported {
            "imported"
        } else {
            "already_present"
        },
        pet: result.pet,
        imported: result.imported,
    };
    if options.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "{}: {} ({})",
            if output.imported {
                "Imported"
            } else {
                "Already present"
            },
            output.pet.manifest.display_name,
            output.pet.pet_ref.0
        );
    }
    Ok(())
}

fn print_preview(output: PreviewOutput, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    println!("Pet import preview");
    println!("  Name: {}", output.manifest.display_name);
    println!(
        "  Version: {}",
        output.manifest.sprite_version_number.number()
    );
    println!("  Dimensions: {}x{}", output.width, output.height);
    println!("  Package hash: {}", output.package_hash);
    println!(
        "  Can import: {}",
        if output.can_commit { "yes" } else { "no" }
    );
    for issue in output.issues {
        println!("  [{:?}] {}: {}", issue.severity, issue.code, issue.message);
    }
    Ok(())
}

fn print_capabilities(json: bool) -> Result<()> {
    let output = CapabilitiesOutput {
        status: "capabilities",
        schema_version: 1,
        hope_agent_version: env!("CARGO_PKG_VERSION"),
        source_kinds: ["directory", "zip", "manifest", "atlas", "httpsArtifact"],
        repeated_local_source: true,
        expected_package_hash: true,
        activate_installed: true,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "Hope Agent pet import protocol {} (Hope Agent {})",
            output.schema_version, output.hope_agent_version
        );
    }
    Ok(())
}

fn request_from_options(options: &ImportOptions) -> Result<PetImportPreviewRequest> {
    let source = if options.sources.len() == 1 {
        let value = &options.sources[0];
        if value.contains("://") {
            let parsed = url::Url::parse(value).context("pet_cli_source_invalid")?;
            if !matches!(parsed.scheme(), "https" | "codex" | "hope-agent") {
                anyhow::bail!("pet_cli_source_scheme_unsupported");
            }
            PetImportSource::Link {
                link: value.clone(),
            }
        } else {
            let path = expand_home(value);
            PetImportSource::LocalPath {
                path: path.to_string_lossy().to_string(),
            }
        }
    } else {
        if options.sources.iter().any(|value| value.contains("://")) {
            anyhow::bail!("pet_cli_multiple_sources_must_be_local");
        }
        PetImportSource::LocalPaths {
            paths: options
                .sources
                .iter()
                .map(|value| expand_home(value).to_string_lossy().to_string())
                .collect(),
        }
    };
    Ok(PetImportPreviewRequest {
        source,
        display_name: options.display_name.clone(),
    })
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn validate_package_hash(value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("blake3:") else {
        anyhow::bail!("pet_cli_expected_package_hash_invalid");
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("pet_cli_expected_package_hash_invalid");
    }
    Ok(())
}

fn parse_command(args: &[String]) -> Result<Command> {
    if args.is_empty()
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok(Command::Help);
    }
    match args[0].as_str() {
        "capabilities" => {
            let json = parse_json_only(&args[1..], "pet capabilities")?;
            Ok(Command::Capabilities { json })
        }
        "list" => {
            let json = parse_json_only(&args[1..], "pet list")?;
            Ok(Command::List { json })
        }
        "activate" => Ok(Command::Activate(parse_activate_options(&args[1..])?)),
        "preview" => {
            let (options, expected) = parse_import_options(&args[1..])?;
            if expected.is_some() {
                anyhow::bail!("--expected-package-hash is only valid for `pet import`");
            }
            Ok(Command::Preview(options))
        }
        "import" => {
            let (options, expected_package_hash) = parse_import_options(&args[1..])?;
            let expected_package_hash = expected_package_hash.ok_or_else(|| {
                anyhow::anyhow!("`pet import` requires --expected-package-hash from `pet preview`")
            })?;
            validate_package_hash(&expected_package_hash)?;
            Ok(Command::Import {
                options,
                expected_package_hash,
            })
        }
        other => anyhow::bail!(
            "unknown pet command `{other}`; expected capabilities | activate | list | preview | import"
        ),
    }
}

fn parse_activate_options(args: &[String]) -> Result<ActivateOptions> {
    let mut pet_ref = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--pet-ref" => set_option(
                &mut pet_ref,
                option_value(args, &mut index, "--pet-ref")?,
                "--pet-ref",
            )?,
            "--json" if !json => json = true,
            other => anyhow::bail!("unknown or duplicate pet activate option `{other}`"),
        }
        index += 1;
    }
    let pet_ref = PetRef(pet_ref.ok_or_else(|| anyhow::anyhow!("--pet-ref is required"))?);
    if !pet_ref.is_well_formed() {
        anyhow::bail!("pet_ref_invalid");
    }
    Ok(ActivateOptions { pet_ref, json })
}

fn parse_json_only(args: &[String], command: &str) -> Result<bool> {
    let mut json = false;
    for arg in args {
        if arg == "--json" && !json {
            json = true;
        } else {
            anyhow::bail!("unknown or duplicate {command} option `{arg}`");
        }
    }
    Ok(json)
}

fn parse_import_options(args: &[String]) -> Result<(ImportOptions, Option<String>)> {
    let mut sources = Vec::new();
    let mut display_name = None;
    let mut expected_package_hash = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                if sources.len() >= MAX_CLI_SOURCES {
                    anyhow::bail!("too many --source values (max {MAX_CLI_SOURCES})");
                }
                sources.push(option_value(args, &mut index, "--source")?);
            }
            "--display-name" => set_option(
                &mut display_name,
                option_value(args, &mut index, "--display-name")?,
                "--display-name",
            )?,
            "--expected-package-hash" => set_option(
                &mut expected_package_hash,
                option_value(args, &mut index, "--expected-package-hash")?,
                "--expected-package-hash",
            )?,
            "--json" if !json => json = true,
            other => anyhow::bail!("unknown or duplicate pet import option `{other}`"),
        }
        index += 1;
    }
    if sources.is_empty() {
        anyhow::bail!("at least one --source is required");
    }
    Ok((
        ImportOptions {
            sources,
            display_name,
            json,
        },
        expected_package_hash,
    ))
}

fn option_value(args: &[String], index: &mut usize, flag: &str) -> Result<String> {
    *index += 1;
    args.get(*index)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

fn set_option(target: &mut Option<String>, value: String, flag: &str) -> Result<()> {
    if target.replace(value).is_some() {
        anyhow::bail!("duplicate option {flag}");
    }
    Ok(())
}

fn print_help() {
    println!("Hope Agent pet package management");
    println!();
    println!("Usage:");
    println!("  hope-agent pet capabilities [--json]");
    println!("  hope-agent pet activate --pet-ref <PET_REF> [--json]");
    println!("  hope-agent pet list [--json]");
    println!(
        "  hope-agent pet preview --source <PATH|URL> [--source <PATH> ...] [--display-name NAME] [--json]"
    );
    println!(
        "  hope-agent pet import --source <PATH|URL> [--source <PATH> ...] --expected-package-hash <BLAKE3> [--display-name NAME] [--json]"
    );
    println!();
    println!("SOURCE may be a local folder, zip, manifest, atlas, or a direct HTTPS");
    println!("zip / manifest / atlas URL from any public origin. Repeat --source for loose");
    println!("local manifest + sprite files from one directory. HTML pages are not packages.");
    println!();
    println!("`preview` never installs. Review its packageHash, then pass that exact hash to");
    println!("`import`; the source is revalidated and a changed package fails closed.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn preview_requires_explicit_source() {
        assert!(parse_command(&strings(&["preview", "--json"])).is_err());
    }

    #[test]
    fn import_requires_hash_from_preview() {
        assert!(parse_command(&strings(&[
            "import",
            "--source",
            "https://example.com/spritesheet.webp"
        ]))
        .is_err());
    }

    #[test]
    fn import_parser_preserves_source_and_stale_guard() {
        let hash = format!("blake3:{}", "a".repeat(64));
        let command = parse_command(&strings(&[
            "import",
            "--json",
            "--source",
            "https://codex-pet.org/zh/pets/ikkun/",
            "--expected-package-hash",
            &hash,
        ]))
        .unwrap();
        assert_eq!(
            command,
            Command::Import {
                options: ImportOptions {
                    sources: vec!["https://codex-pet.org/zh/pets/ikkun/".to_string()],
                    display_name: None,
                    json: true,
                },
                expected_package_hash: hash,
            }
        );
    }

    #[test]
    fn unsupported_url_scheme_never_becomes_a_local_path() {
        let options = ImportOptions {
            sources: vec!["http://example.com/pet.webp".to_string()],
            display_name: None,
            json: true,
        };
        assert!(request_from_options(&options).is_err());
    }

    #[test]
    fn arbitrary_https_package_origins_use_the_shared_link_pipeline() {
        let options = ImportOptions {
            sources: vec!["https://downloads.example/community/pet-package.zip".to_string()],
            display_name: None,
            json: true,
        };
        let request = request_from_options(&options).unwrap();
        assert!(matches!(
            request.source,
            PetImportSource::Link { link }
                if link == "https://downloads.example/community/pet-package.zip"
        ));
    }

    #[test]
    fn repeated_local_sources_use_the_loose_file_pipeline() {
        let command = parse_command(&strings(&[
            "preview",
            "--source",
            "/tmp/pet/pet.json",
            "--source",
            "/tmp/pet/spritesheet.webp",
            "--json",
        ]))
        .unwrap();
        let Command::Preview(options) = command else {
            panic!("expected preview command");
        };
        let request = request_from_options(&options).unwrap();
        assert!(matches!(
            request.source,
            PetImportSource::LocalPaths { paths }
                if paths == vec!["/tmp/pet/pet.json", "/tmp/pet/spritesheet.webp"]
        ));
    }

    #[test]
    fn capability_probe_has_a_stable_machine_marker() {
        assert_eq!(
            parse_command(&strings(&["capabilities", "--json"])).unwrap(),
            Command::Capabilities { json: true }
        );
    }

    #[test]
    fn activate_requires_a_well_formed_pet_ref() {
        assert_eq!(
            parse_command(&strings(&[
                "activate",
                "--pet-ref",
                "custom:moon-cat",
                "--json"
            ]))
            .unwrap(),
            Command::Activate(ActivateOptions {
                pet_ref: PetRef("custom:moon-cat".to_string()),
                json: true,
            })
        );
        assert!(parse_command(&strings(&["activate", "--pet-ref", "moon-cat"])).is_err());
    }

    #[test]
    fn desktop_activate_url_uses_loopback_for_unspecified_bind() {
        assert_eq!(
            desktop_pet_activate_url("0.0.0.0:8420").unwrap().as_str(),
            "http://127.0.0.1:8420/api/pets/activate"
        );
        assert_eq!(
            desktop_pet_activate_url("[::]:8420").unwrap().as_str(),
            "http://[::1]:8420/api/pets/activate"
        );
        assert!(desktop_pet_activate_url("192.0.2.10:8420").is_err());
    }
}
