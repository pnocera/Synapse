use std::sync::Arc;

use crate::{
    ActionBackend, HardwareUnavailableBackend, ResolvedBackend, VigemBackend,
    backend::software::SoftwareBackend,
};

pub struct Backends {
    software: Arc<dyn ActionBackend>,
    vigem: Arc<dyn ActionBackend>,
    hardware: Arc<dyn ActionBackend>,
}

impl Backends {
    #[must_use]
    pub fn production() -> Self {
        Self {
            software: Arc::new(SoftwareBackend::new()),
            vigem: Arc::new(VigemBackend::new()),
            hardware: Arc::new(HardwareUnavailableBackend::new()),
        }
    }

    #[must_use]
    pub fn all_routed_to(backend: Arc<dyn ActionBackend>) -> Self {
        Self {
            software: Arc::clone(&backend),
            vigem: Arc::clone(&backend),
            hardware: backend,
        }
    }

    pub(super) fn pick(&self, resolved: ResolvedBackend) -> Arc<dyn ActionBackend> {
        match resolved {
            ResolvedBackend::Software => Arc::clone(&self.software),
            ResolvedBackend::Vigem => Arc::clone(&self.vigem),
            ResolvedBackend::Hardware => Arc::clone(&self.hardware),
        }
    }

    pub(super) fn pick_vigem_if_distinct_from(
        &self,
        backend: &Arc<dyn ActionBackend>,
    ) -> Option<Arc<dyn ActionBackend>> {
        (!Arc::ptr_eq(backend, &self.vigem)).then(|| Arc::clone(&self.vigem))
    }
}

impl std::fmt::Debug for Backends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backends").finish_non_exhaustive()
    }
}
