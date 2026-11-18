// 앱이 띄운 Windows 자식 프로세스를 종료 시 자동으로 회수한다.
use std::{
    io,
    process::{Child, Command},
};

#[cfg(windows)]
pub(crate) struct Owner(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Owner)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let owner = Owner(handle);
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut child = command.spawn()?;
    if unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } == 0 {
        let error = io::Error::last_os_error();
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    Ok((child, owner))
}
#[cfg(not(windows))]
pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, ())> {
    Ok((command.spawn()?, ()))
}
