use std::process::Child;

use open_print_pdf_core::Result;

#[cfg(windows)]
use open_print_pdf_core::OpenPrintPdfError;

#[cfg(windows)]
pub(crate) const WINDOWS_PROCESS_MEMORY_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[cfg(not(windows))]
pub(crate) struct ProcessSandbox;

#[cfg(not(windows))]
pub(crate) fn attach(_child: &Child) -> Result<ProcessSandbox> {
    Ok(ProcessSandbox)
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    use super::*;

    pub(crate) struct ProcessSandbox {
        job: HANDLE,
    }

    impl Drop for ProcessSandbox {
        fn drop(&mut self) {
            // SAFETY: `job` is an owned handle returned by CreateJobObjectW and is closed once.
            unsafe {
                CloseHandle(self.job);
            }
        }
    }

    pub(crate) fn attach(child: &Child) -> Result<ProcessSandbox> {
        // SAFETY: A null security descriptor and name request a private job object.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(last_error("could not create Windows Job Object"));
        }
        let guard = ProcessSandbox { job };
        // SAFETY: This C-compatible structure is valid when zero-initialized, and only documented
        // fields selected by LimitFlags are read by SetInformationJobObject.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = WINDOWS_PROCESS_MEMORY_LIMIT_BYTES;
        // SAFETY: `guard.job` is valid and `limits` points to the correctly sized information
        // structure for JobObjectExtendedLimitInformation for the duration of the call.
        let configured = unsafe {
            SetInformationJobObject(
                guard.job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(last_error("could not configure Windows Job Object"));
        }
        let process = child.as_raw_handle() as HANDLE;
        // SAFETY: `guard.job` and the Child process handle are valid owned/borrowed handles.
        if unsafe { AssignProcessToJobObject(guard.job, process) } == 0 {
            return Err(last_error(
                "could not attach Ghostscript to Windows Job Object",
            ));
        }
        Ok(guard)
    }

    fn last_error(context: &str) -> OpenPrintPdfError {
        OpenPrintPdfError::ConversionFailed(format!(
            "{context}: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(windows)]
pub(crate) use windows::attach;
