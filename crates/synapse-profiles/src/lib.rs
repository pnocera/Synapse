pub mod error;
pub mod package;
pub mod parser;
pub mod resolver;
mod toml_format;
pub mod watcher;

pub use error::{ProfileError, ProfileLoadError};
pub use package::{
    PROFILE_PACKAGE_KIND, PROFILE_PACKAGE_SCHEMA_VERSION, PackageAssumptions, PackageAuthor,
    PackageChangelogEntry, PackageContributionPermissions, PackageDependency,
    PackageExecutionPermissions, PackageFiles, PackageHashes, PackageInput, PackagePermissions,
    PackageSignature, PackageSource, PackageTarget, PackageTrust, ProfilePackageManifest,
    package_manifest_digest, package_signature_payload, package_signature_payload_digest,
    parse_package_manifest_bytes, parse_package_manifest_bytes_with_digest,
    parse_package_manifest_file,
};
pub use parser::{
    LoadedProfile, ProfileDefaults, ScreenBounds, bundled_profiles_dir, parse_profile_bytes,
    parse_profile_file, parse_profile_file_with_bounds,
};
pub use resolver::{ForegroundWindow, ProfileMatchResolution, resolve_active_profile};
pub use watcher::{
    ForegroundProfileTransition, ProfileEventExtensionStatus, ProfileRuntime, ProfileStatus,
};
