//! Stack policy shared by guest callers and executable link settings.

#[path = "guest_stack_constants.rs"]
mod constants;
pub use constants::{GUEST_RESERVATION_BYTES, GUEST_STACK_BYTES, HOST_HEADROOM};

/// Spawn guarantees the reservation even on platforms without a stack-size query.
pub fn builder() -> std::thread::Builder {
    std::thread::Builder::new().stack_size(GUEST_RESERVATION_BYTES)
}

/// The refusal message quotes no figure: this build reserves 136 MiB on a 64-bit process and
/// 18 MiB on a 32-bit one, and the caller only needs to know the stack is too small.
fn too_small() -> &'static str {
    "GuestStackUnavailable: calling thread stack is too small"
}

/// Starts a thread that may enter a guest: guest-sized and named `name`, and in debug builds it
/// asserts at its start that the reservation took, so a test that starts it fails loudly if not.
pub fn spawn<T: Send + 'static>(
    name: String,
    body: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<T>> {
    builder().name(name).spawn(move || {
        debug_assert_eq!(
            check_current(),
            Ok(()),
            "a guest-capable thread must start on the guest stack"
        );
        body()
    })
}

/// Runs `test` on a guest-sized worker named like the calling thread, so libtest reports it under
/// its own name, and resumes its panic with the original payload. For tests that enter a guest:
/// libtest's own threads are too small.
pub fn run_on_guest_stack(test: impl FnOnce() + Send + 'static) {
    let mut worker = builder();
    if let Some(name) = std::thread::current().name() {
        worker = worker.name(name.to_owned());
    }
    if let Err(payload) = worker.spawn(test).unwrap().join() {
        std::panic::resume_unwind(payload);
    }
}

/// Must run before compiler-child handling or any game initialization.
pub fn prepare_main() {
    if let Err(error) = prepare() {
        eprintln!("GuestStackUnavailable: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare() -> std::io::Result<()> {
    // Executable link settings reserve the same stack on Windows and macOS.
    inspect_current().map_err(std::io::Error::other)
}

#[cfg(target_os = "linux")]
fn prepare() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let required = (GUEST_STACK_BYTES + HOST_HEADROOM) as libc::rlim_t;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is writable and getrlimit initializes it on success.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if limit.rlim_cur >= required && inspect_current().is_ok() {
        return Ok(());
    }
    let describe = |bytes| {
        if bytes == libc::RLIM_INFINITY {
            "unlimited".to_owned()
        } else {
            format!("{} KiB", bytes / 1024)
        }
    };
    let limits = format!(
        "soft={}, hard={}",
        describe(limit.rlim_cur),
        describe(limit.rlim_max)
    );
    if std::env::var_os("HORIZON_STACK_REEXEC").is_some() {
        return Err(std::io::Error::other(format!(
            "stack re-exec marker already set but the stack is insufficient ({limits})"
        )));
    }
    if limit.rlim_max < required {
        return Err(std::io::Error::other(format!(
            "stack hard limit is too low ({limits}); raise ulimit -Hs to at least {} KiB",
            required.saturating_add(HOST_HEADROOM as libc::rlim_t) / 1024
        )));
    }
    // pthread_getattr_np excludes the initial argument/environment pages from
    // its reported stack extent. Reserve additional host headroom for those
    // pages when the hard limit permits it; guest entry still verifies the extent.
    limit.rlim_cur = required
        .saturating_add(HOST_HEADROOM as libc::rlim_t)
        .min(limit.rlim_max);
    // SAFETY: limit is initialized; this changes only this process's resource limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_STACK, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Linux chooses its mmap base at exec using RLIMIT_STACK. Re-exec keeps the
    // enlarged stack's address range clear before libraries and workers start.
    Err(std::process::Command::new(std::env::current_exe()?)
        .arg0(std::env::args_os().next().unwrap_or_default())
        .args(std::env::args_os().skip(1))
        .env("HORIZON_STACK_REEXEC", "1")
        .exec())
}

/// Refuse guest entry when the calling thread's stack reservation is too small.
/// Cache the platform query once per thread; the hot path reads only thread-local state.
pub fn check_current() -> Result<(), &'static str> {
    thread_local! {
        static CHECK: std::cell::OnceCell<Result<(), &'static str>> = const { std::cell::OnceCell::new() };
    }
    CHECK.with(|check| *check.get_or_init(inspect_current))
}

fn inspect_current() -> Result<(), &'static str> {
    #[cfg(target_os = "linux")]
    {
        let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
        // SAFETY: pthread_self is live and attr is writable. Destroy only on success.
        unsafe {
            if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
                return Err("GuestStackUnavailable: cannot inspect the calling thread stack");
            }
            let mut attr = attr.assume_init();
            let mut size = 0;
            let result = libc::pthread_attr_getstacksize(&attr, &mut size);
            libc::pthread_attr_destroy(&mut attr);
            if result != 0 || size < GUEST_RESERVATION_BYTES {
                return Err(too_small());
            }
        }
    }
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: pthread_self names the live calling thread.
        let size = unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) };
        if size < GUEST_RESERVATION_BYTES {
            return Err(too_small());
        }
    }
    #[cfg(windows)]
    {
        if let Some((low, high)) = stack_bounds() {
            if high.saturating_sub(low) < GUEST_RESERVATION_BYTES {
                return Err(too_small());
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
    return Err("GuestStackUnavailable: stack inspection is unsupported on this platform");
    #[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
    Ok(())
}

/// The calling thread's stack bounds, `(low, high)` in address order.
///
/// `GetCurrentThreadStackLimits` arrived in Windows 8, and a *static* import of it keeps the
/// image from loading on Windows 7 at all: the loader reports "entry point
/// GetCurrentThreadStackLimits not found in KERNEL32.dll" before `main`. So it is resolved at run
/// time, and on Windows 7 the reservation is measured instead: every stack is one reserved
/// allocation, and `VirtualQuery` walks its regions up to the allocation's top.
#[cfg(windows)]
fn stack_bounds() -> Option<(usize, usize)> {
    type Limits = unsafe extern "system" fn(*mut usize, *mut usize);
    // SAFETY: constant name strings, and the returned address is only called when non-null.
    let module = unsafe { GetModuleHandleA(kernel32.as_ptr()) };
    let limits = if module.is_null() {
        None
    } else {
        // SAFETY: `module` is a live module handle and the name is NUL-terminated.
        unsafe { GetProcAddress(module, stack_limits.as_ptr()) }
    };
    if let Some(address) = limits {
        // SAFETY: kernel32 exports `GetCurrentThreadStackLimits` with exactly this signature.
        let limits: Limits = unsafe { std::mem::transmute(address) };
        let (mut low, mut high) = (0, 0);
        // SAFETY: both pointers refer to writable stack bounds.
        unsafe { limits(&mut low, &mut high) };
        return Some((low, high));
    }
    reservation_bounds()
}

#[cfg(windows)]
const kernel32: &[u8] = b"kernel32.dll\0";
#[cfg(windows)]
const stack_limits: &[u8] = b"GetCurrentThreadStackLimits\0";

#[cfg(windows)]
#[repr(C)]
struct MemoryBasicInformation {
    base_address: usize,
    allocation_base: usize,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut std::ffi::c_void;
    fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> Option<unsafe extern "system" fn()>;
    fn VirtualQuery(
        address: usize,
        buffer: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;
}

/// The whole reserved stack allocation around a live local, low address first.
#[cfg(windows)]
fn reservation_bounds() -> Option<(usize, usize)> {
    let size = std::mem::size_of::<MemoryBasicInformation>();
    let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::uninit();
    let marker = 0u8;
    // SAFETY: `info` is writable for `size` bytes, `marker` is a live stack address, and
    // `VirtualQuery` is a Windows XP-era export that every Windows 7 has.
    let queried = unsafe { VirtualQuery((&marker as *const u8) as usize, info.as_mut_ptr(), size) };
    if queried != size {
        return None;
    }
    // SAFETY: VirtualQuery filled the whole structure.
    let base = unsafe { info.assume_init() }.allocation_base;
    let mut high = base;
    // A stack's pages are one allocation: walk until the allocation changes or memory runs out.
    for _ in 0..4096 {
        let mut region = std::mem::MaybeUninit::<MemoryBasicInformation>::uninit();
        // SAFETY: `region` is writable for `size` bytes; `high` is inside the allocation.
        let queried = unsafe { VirtualQuery(high, region.as_mut_ptr(), size) };
        if queried != size {
            break;
        }
        // SAFETY: VirtualQuery filled the whole structure.
        let region = unsafe { region.assume_init() };
        if region.allocation_base != base || region.region_size == 0 {
            break;
        }
        high = high.checked_add(region.region_size)?;
    }
    (high > base).then_some((base, high))
}

#[cfg(test)]
#[test]
fn audited_guest_workers_use_the_stack_policy() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [
        "crates/horizon-threads/src/jobs.rs",
        "crates/horizon-threads/src/lib.rs",
        "crates/horizon-client/src/lan/io_pool.rs",
        "crates/horizon-client/src/vanillagen.rs",
        "crates/horizon-client/src/vanillagen/lod.rs",
        "crates/horizon-client/src/game/runtime/resource_load.rs",
        "crates/horizon-client/src/game/interface/startup_menu.rs",
        "crates/horizon-client/src/game/world/world_loading.rs",
        "crates/horizon-client/src/game/world/world_entry.rs",
        "crates/horizon-client/src/game/world/packs/mod.rs",
    ] {
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let source = source.split("\n#[cfg(test)]").next().unwrap();
        assert!(
            ![
                "thread::Builder::new()",
                "thread::spawn(",
                "scope.spawn(",
                "s.spawn("
            ]
            .iter()
            .any(|form| source.contains(form)),
            "{file}: route guest workers through horizon_guest_stack::builder"
        );
    }
    let server =
        std::fs::read_to_string(root.join("crates/horizon-client/src/game/world/integrated.rs"))
            .unwrap();
    // Production code only: a test may start its own server thread with the same policy.
    let server = server.split("\n#[cfg(test)]").next().unwrap();
    assert_eq!(
        server.matches(".stack_size(server_thread_stack())").count(),
        2
    );
    assert_eq!(server.matches("thread::Builder::new()").count(), 2);
    for form in ["thread::spawn(", "scope.spawn(", "s.spawn("] {
        assert!(
            !server.contains(form),
            "server: route guest workers through the stack policy"
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn small_worker_refuses_guest_entry() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                assert!(
                    super::check_current()
                        .unwrap_err()
                        .contains("GuestStackUnavailable")
                )
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn guest_worker_reserves_the_required_stack() {
        super::builder()
            .spawn(|| super::check_current().unwrap())
            .unwrap()
            .join()
            .unwrap();
    }
}
