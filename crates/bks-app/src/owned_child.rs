//! Own only a freshly spawned process and its descendants in a private job.
//! Spawn and destruction can wait; callers must keep both off the UI thread.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};

use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    GetProcessIdOfThread, OpenThread, ResumeThread, CREATE_SUSPENDED,
    THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// This job contains only the process spawned by this transport and its
/// descendants. Closing it never searches for or kills an existing Codex.
pub(crate) struct OwnedChild {
    pub(crate) child: Child,
    job: HANDLE,
}

impl OwnedChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        // SAFETY: the job is unnamed and its handle remains private to this
        // worker. Only this freshly spawned child's live handle is assigned.
        unsafe {
            let job = CreateJobObjectW(None, None).map_err(io::Error::other)?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Err(error) = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) {
                let _ = CloseHandle(job);
                return Err(io::Error::other(error));
            }
            // The primary thread cannot execute user code (or spawn escaping
            // descendants) until the job is assigned and explicitly resumed.
            let mut child = match command
                .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED.0)
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    let _ = CloseHandle(job);
                    return Err(error);
                }
            };
            if let Err(error) = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = CloseHandle(job);
                return Err(io::Error::other(error));
            }
            let owned = Self { child, job };
            resume_primary_thread(&owned.child)?;
            Ok(owned)
        }
    }
}

pub(crate) struct ScopedHandle(pub(crate) HANDLE);

impl Drop for ScopedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn resume_primary_thread(child: &Child) -> io::Result<()> {
    // Stable Rust 1.99 does not expose Child's primary thread handle. For a
    // freshly CREATE_SUSPENDED child, require exactly one owned thread in the
    // documented Toolhelp snapshot, then re-check its process identity before
    // calling the documented ResumeThread API. Never resume another process.
    unsafe {
        let snapshot =
            ScopedHandle(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(io::Error::other)?);
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        Thread32First(snapshot.0, &mut entry).map_err(io::Error::other)?;
        let mut primary_id = None;
        loop {
            if entry.dwSize != std::mem::size_of::<THREADENTRY32>() as u32 {
                return Err(io::Error::other("Неизвестная схема Toolhelp thread entry."));
            }
            if entry.th32OwnerProcessID == child.id()
                && primary_id.replace(entry.th32ThreadID).is_some()
            {
                return Err(io::Error::other(
                    "Созданный suspended child имеет неоднозначный primary thread.",
                ));
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            match Thread32Next(snapshot.0, &mut entry) {
                Ok(()) => {}
                Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
                Err(error) => return Err(io::Error::other(error)),
            }
        }
        let id = primary_id.ok_or_else(|| {
            io::Error::other("Не найден primary thread собственного suspended child.")
        })?;
        let primary = ScopedHandle(
            OpenThread(
                THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                false,
                id,
            )
            .map_err(io::Error::other)?,
        );
        if GetProcessIdOfThread(primary.0) != child.id() {
            return Err(io::Error::other(
                "Изменился владелец primary thread. Resume отменён.",
            ));
        }
        let previous = ResumeThread(primary.0);
        if previous == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if previous != 1 {
            return Err(io::Error::other(
                "Неожиданное suspend count собственного child.",
            ));
        }
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // Closing the job first also releases descendant-owned pipe handles,
        // allowing an otherwise blocked stdin writer to finish off the UI.
        unsafe {
            let _ = CloseHandle(self.job);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
