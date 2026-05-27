use sha2::{Digest, Sha256};
use synapse_core::{Backend, ProfileUseScope};

use super::types::{PackageDependency, PackageTarget, ProfilePackageManifest};

#[must_use]
pub fn package_manifest_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    format!("sha256:{}", hex_lower(&digest))
}

pub fn same_digest(left: &str, right: &str) -> bool {
    left.strip_prefix("sha256:")
        .zip(right.strip_prefix("sha256:"))
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "signature payload must enumerate every signed manifest field in one visible order"
)]
pub fn package_signature_payload(manifest: &ProfilePackageManifest) -> Vec<u8> {
    let mut lines = vec![
        "synapse.profile_package.signature.v1".to_owned(),
        format!("schema_version={}", manifest.schema_version),
        format!("kind={}", manifest.kind),
        format!("package_id={}", manifest.package_id),
        format!("package_version={}", manifest.package_version),
        format!("profile_id={}", manifest.profile_id),
        format!("profile_version={}", manifest.profile_version),
        format!("created_at={}", manifest.created_at),
        format!("author.name={}", manifest.author.name),
        format!("author.contact={}", manifest.author.contact),
        format!("author.attribution={}", manifest.author.attribution),
        format!("source.kind={}", manifest.source.kind),
        format!("source.uri={}", manifest.source.uri),
        format!("source.revision={}", manifest.source.revision),
        format!("source.built_by={}", manifest.source.built_by),
        format!("source.generated_by={}", manifest.source.generated_by),
    ];
    push_targets(&mut lines, &manifest.targets);
    lines.push(format!("assumptions.os={}", manifest.assumptions.os));
    lines.push(format!(
        "assumptions.synapse_schema_version={}",
        manifest.assumptions.synapse_schema_version
    ));
    for benchmark_id in &manifest.assumptions.benchmark_ids {
        lines.push(format!("assumptions.benchmark_id={benchmark_id}"));
    }
    lines.push(format!(
        "assumptions.display.min_width={}",
        manifest.assumptions.display.min_width
    ));
    lines.push(format!(
        "assumptions.display.min_height={}",
        manifest.assumptions.display.min_height
    ));
    lines.push(format!(
        "assumptions.display.dpi_scale_min={}",
        manifest.assumptions.display.dpi_scale_min
    ));
    lines.push(format!(
        "assumptions.display.dpi_scale_max={}",
        manifest.assumptions.display.dpi_scale_max
    ));
    for backend in &manifest.input.backends {
        lines.push(format!("input.backend={}", backend_label(*backend)));
    }
    push_dependencies(&mut lines, "input.firmware", &manifest.input.firmware);
    push_dependencies(&mut lines, "input.models", &manifest.input.models);
    lines.push(format!(
        "permissions.license_spdx={}",
        manifest.permissions.license_spdx
    ));
    lines.push(format!(
        "permissions.contribution_terms={}",
        manifest.permissions.contribution_terms
    ));
    lines.push(format!(
        "permissions.use_scope={}",
        use_scope_label(manifest.permissions.use_scope)
    ));
    lines.push(format!(
        "permissions.execution.local_only={}",
        manifest.permissions.execution.local_only
    ));
    lines.push(format!(
        "permissions.execution.remote_server_allowed={}",
        manifest.permissions.execution.remote_server_allowed
    ));
    lines.push(format!(
        "permissions.contribution.export_allowed={}",
        manifest.permissions.contribution.export_allowed
    ));
    lines.push(format!(
        "permissions.contribution.share_audit_allowed={}",
        manifest.permissions.contribution.share_audit_allowed
    ));
    for entry in &manifest.changelog {
        lines.push(format!("changelog.version={}", entry.version));
        lines.push(format!("changelog.at={}", entry.at));
        lines.push(format!("changelog.summary={}", entry.summary));
    }
    lines.push(format!(
        "hashes.profile_toml_sha256={}",
        manifest.hashes.profile_toml_sha256
    ));
    lines.push(format!(
        "hashes.package_sha256={}",
        manifest.hashes.package_sha256
    ));
    for (asset, digest) in &manifest.hashes.assets {
        lines.push(format!("hashes.assets.{asset}={digest}"));
    }
    lines.push(format!(
        "files.profile_toml={}",
        manifest.files.profile_toml
    ));
    for asset in &manifest.files.assets {
        lines.push(format!("files.asset={asset}"));
    }
    lines.push(format!("trust.policy={}", manifest.trust.policy));
    for signer in &manifest.trust.required_signers {
        lines.push(format!("trust.required_signer={signer}"));
    }
    for (key, value) in &manifest.metadata {
        lines.push(format!("metadata.{key}={value}"));
    }
    lines.push(String::new());
    lines.join("\n").into_bytes()
}

#[must_use]
pub fn package_signature_payload_digest(manifest: &ProfilePackageManifest) -> String {
    package_manifest_digest(&package_signature_payload(manifest))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn push_targets(lines: &mut Vec<String>, targets: &[PackageTarget]) {
    for target in targets {
        lines.push(format!("target.target_id={}", target.target_id));
        lines.push(format!("target.target_kind={}", target.target_kind));
        push_optional(lines, "target.app_id", target.app_id.as_deref());
        push_optional(lines, "target.process_name", target.process_name.as_deref());
        push_optional(lines, "target.title_regex", target.title_regex.as_deref());
        if let Some(steam_appid) = target.steam_appid {
            lines.push(format!("target.steam_appid={steam_appid}"));
        }
        push_optional(lines, "target.app_version", target.app_version.as_deref());
    }
}

fn push_dependencies(lines: &mut Vec<String>, prefix: &str, dependencies: &[PackageDependency]) {
    for dependency in dependencies {
        lines.push(format!("{prefix}.id={}", dependency.id));
        lines.push(format!("{prefix}.version={}", dependency.version));
        push_optional(
            lines,
            &format!("{prefix}.digest"),
            dependency.digest.as_deref(),
        );
    }
}

fn push_optional(lines: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        lines.push(format!("{name}={value}"));
    }
}

const fn backend_label(backend: Backend) -> &'static str {
    match backend {
        Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
        Backend::Auto => "auto",
    }
}

const fn use_scope_label(scope: ProfileUseScope) -> &'static str {
    match scope {
        ProfileUseScope::Productivity => "productivity",
        ProfileUseScope::SinglePlayer => "single_player",
        ProfileUseScope::OperatorOwnedTest => "operator_owned_test",
        ProfileUseScope::SanctionedResearch => "sanctioned_research",
        ProfileUseScope::Unknown => "unknown",
    }
}
