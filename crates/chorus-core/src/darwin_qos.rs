//! Darwin Thread Quality of Service (QoS) & Apple Silicon Core Scheduling.
//!
//! Maps Rust threads to Apple Silicon P-cores (Performance) vs E-cores (Efficiency):
//! - `UserInitiated`: Routes to P-cores for AST parsing, diff computation, and token routing.
//! - `Utility`: Routes to E-cores for SQLite WAL checkpoints, FTS5 indexing, and background sync.

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarwinQoSClass {
    UserInteractive = 0x21,
    UserInitiated   = 0x19,
    Default         = 0x15,
    Utility         = 0x11,
    Background      = 0x09,
}

#[cfg(target_os = "macos")]
mod sys {
    use std::os::raw::c_int;

    extern "C" {
        pub fn pthread_set_qos_class_self_np(
            qos_class: u32,
            relative_priority: c_int,
        ) -> c_int;
    }
}

pub struct DarwinQoS;

impl DarwinQoS {
    /// Sets the Darwin scheduler Quality of Service class for the calling thread.
    /// Returns Ok(()) on macOS if successful, or no-op Ok(()) on other operating systems.
    pub fn set_current_thread_qos(class: DarwinQoSClass) -> Result<(), std::io::Error> {
        #[cfg(target_os = "macos")]
        {
            let ret = unsafe { sys::pthread_set_qos_class_self_np(class as u32, 0) };
            if ret != 0 {
                return Err(std::io::Error::from_raw_os_error(ret));
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = class;
        }
        Ok(())
    }

    /// Helper: Sets calling thread to P-cores (UserInitiated) for compute-heavy work.
    pub fn set_p_core_affinity() -> Result<(), std::io::Error> {
        Self::set_current_thread_qos(DarwinQoSClass::UserInitiated)
    }

    /// Helper: Sets calling thread to E-cores (Utility) for battery-efficient background tasks.
    pub fn set_e_core_affinity() -> Result<(), std::io::Error> {
        Self::set_current_thread_qos(DarwinQoSClass::Utility)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_thread_qos_succeeds() {
        assert!(DarwinQoS::set_p_core_affinity().is_ok());
        assert!(DarwinQoS::set_e_core_affinity().is_ok());
    }
}
