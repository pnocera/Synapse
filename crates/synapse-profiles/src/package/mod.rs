mod digest;
mod types;
mod validation;

use std::{fs, path::Path};

use crate::error::ProfileError;

pub use digest::{
    package_manifest_digest, package_signature_payload, package_signature_payload_digest,
};
pub use types::{
    DisplayAssumptions, PackageAssumptions, PackageAuthor, PackageChangelogEntry,
    PackageContributionPermissions, PackageDependency, PackageExecutionPermissions, PackageFiles,
    PackageHashes, PackageInput, PackagePermissions, PackageSignature, PackageSource,
    PackageTarget, PackageTrust, ProfilePackageManifest,
};
pub use validation::{PROFILE_PACKAGE_KIND, PROFILE_PACKAGE_SCHEMA_VERSION};

/// Parses and validates a profile package manifest file.
///
/// # Errors
///
/// Returns [`ProfileError`] when the file cannot be read, decoded as TOML, or
/// validated against the fail-closed package metadata rules.
pub fn parse_package_manifest_file(
    path: impl AsRef<Path>,
) -> Result<ProfilePackageManifest, ProfileError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| ProfileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_package_manifest_bytes(path, &bytes)
}

/// Parses and validates package manifest bytes.
///
/// # Errors
///
/// Returns [`ProfileError`] when TOML decoding fails or validation rejects
/// ambiguous, unsafe, or incompatible metadata.
pub fn parse_package_manifest_bytes(
    path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<ProfilePackageManifest, ProfileError> {
    let path = path.as_ref().to_path_buf();
    let manifest: ProfilePackageManifest =
        toml::from_slice(bytes).map_err(|source| ProfileError::Parse {
            path: path.clone(),
            message: source.to_string(),
        })?;
    manifest.validate(&path)?;
    Ok(manifest)
}

/// Checks a registry-supplied manifest digest before parsing package metadata.
///
/// # Errors
///
/// Returns [`ProfileError`] when the expected digest is malformed, the byte
/// digest differs, or the manifest itself fails validation.
pub fn parse_package_manifest_bytes_with_digest(
    path: impl AsRef<Path>,
    bytes: &[u8],
    expected_manifest_digest: &str,
) -> Result<ProfilePackageManifest, ProfileError> {
    let path = path.as_ref();
    validation::validate_sha256_digest(path, "expected_manifest_digest", expected_manifest_digest)?;
    let actual = package_manifest_digest(bytes);
    if !digest::same_digest(&actual, expected_manifest_digest) {
        return Err(ProfileError::Parse {
            path: path.to_path_buf(),
            message: format!(
                "manifest digest mismatch: expected {expected_manifest_digest}, actual {actual}"
            ),
        });
    }
    parse_package_manifest_bytes(path, bytes)
}
