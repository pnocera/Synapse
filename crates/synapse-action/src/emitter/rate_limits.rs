use crate::{ResolvedBackend, TokenBucket};

#[cfg(test)]
use crate::TokenBucketSnapshot;

#[cfg(test)]
#[derive(Debug)]
pub(super) struct BackendRateLimitSnapshot {
    pub(super) software: TokenBucketSnapshot,
    pub(super) vigem: TokenBucketSnapshot,
    pub(super) hardware: TokenBucketSnapshot,
}

pub(super) struct BackendRateLimits {
    software: TokenBucket,
    vigem: TokenBucket,
    hardware: TokenBucket,
}

impl BackendRateLimits {
    pub(super) fn new() -> Self {
        Self {
            software: TokenBucket::for_backend(ResolvedBackend::Software),
            vigem: TokenBucket::for_backend(ResolvedBackend::Vigem),
            hardware: TokenBucket::for_backend(ResolvedBackend::Hardware),
        }
    }

    #[cfg(test)]
    pub(super) const fn with_buckets(
        software: TokenBucket,
        vigem: TokenBucket,
        hardware: TokenBucket,
    ) -> Self {
        Self {
            software,
            vigem,
            hardware,
        }
    }

    pub(super) const fn bucket(&self, backend: ResolvedBackend) -> &TokenBucket {
        match backend {
            ResolvedBackend::Software => &self.software,
            ResolvedBackend::Vigem => &self.vigem,
            ResolvedBackend::Hardware => &self.hardware,
        }
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> BackendRateLimitSnapshot {
        BackendRateLimitSnapshot {
            software: self.software.snapshot(),
            vigem: self.vigem.snapshot(),
            hardware: self.hardware.snapshot(),
        }
    }
}
