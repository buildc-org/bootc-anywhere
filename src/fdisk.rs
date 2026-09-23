use super::*;
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};

// SAFETY: each declaration matches the libfdisk symbol this crate links, so a call passes arguments of the declared type.
unsafe extern "C" {
    fn fdisk_new_context() -> *mut c_void;
    fn fdisk_unref_context(cxt: *mut c_void);
    fn fdisk_disable_dialogs(cxt: *mut c_void, disable: c_int) -> c_int;
    fn fdisk_assign_device(cxt: *mut c_void, fname: *const c_char, readonly: c_int) -> c_int;
    fn fdisk_deassign_device(cxt: *mut c_void, nosync: c_int) -> c_int;
    fn fdisk_has_label(cxt: *mut c_void) -> c_int;
    fn fdisk_get_label(cxt: *mut c_void, name: *const c_char) -> *mut c_void;
    fn fdisk_label_get_name(label: *mut c_void) -> *const c_char;
    fn fdisk_get_first_lba(cxt: *mut c_void) -> u64;
    fn fdisk_get_last_lba(cxt: *mut c_void) -> u64;
    fn fdisk_get_nsectors(cxt: *mut c_void) -> u64;
    fn fdisk_get_sector_size(cxt: *mut c_void) -> c_ulong;
    fn fdisk_get_partitions(cxt: *mut c_void, table: *mut *mut c_void) -> c_int;
    fn fdisk_unref_table(table: *mut c_void);
    fn fdisk_table_get_nents(table: *mut c_void) -> usize;
    fn fdisk_table_get_partition(table: *mut c_void, n: usize) -> *mut c_void;
    fn fdisk_partition_has_start(part: *mut c_void) -> c_int;
    fn fdisk_partition_has_size(part: *mut c_void) -> c_int;
    fn fdisk_partition_has_partno(part: *mut c_void) -> c_int;
    fn fdisk_partition_get_start(part: *mut c_void) -> u64;
    fn fdisk_partition_get_size(part: *mut c_void) -> u64;
    fn fdisk_partition_get_partno(part: *mut c_void) -> usize;
    fn fdisk_partname(dev: *const c_char, partno: usize) -> *mut c_char;
}

struct Context {
    cxt: *mut c_void,
    assigned: bool,
}

impl Context {
    fn read_only(disk: &str) -> Result<Self, String> {
        let path = CString::new(disk).map_err(|_| format!("{disk}: the path holds a NUL byte"))?;
        // SAFETY: fdisk_new_context takes no argument and returns an owned context or null, which is checked below.
        let cxt = unsafe { fdisk_new_context() };
        if cxt.is_null() {
            return Err(format!("libfdisk: no context for {disk}"));
        }
        let mut context = Self {
            cxt,
            assigned: false,
        };
        // SAFETY: `context.cxt` is a live context from `fdisk_new_context`.
        unsafe { fdisk_disable_dialogs(context.cxt, 1) };
        // SAFETY: `context.cxt` is live and `path` is a NUL-terminated string that outlives the call.
        let rc = unsafe { fdisk_assign_device(context.cxt, path.as_ptr(), 1) };
        if rc < 0 {
            return Err(format!("libfdisk: cannot read {disk} ({rc})"));
        }
        context.assigned = true;
        Ok(context)
    }

    fn label(&self) -> String {
        // SAFETY: `self.cxt` is a live context owned by `Self`.
        if unsafe { fdisk_has_label(self.cxt) } == 0 {
            return String::new();
        }
        // SAFETY: `self.cxt` is live and a null name asks for its assigned label.
        let label = unsafe { fdisk_get_label(self.cxt, std::ptr::null()) };
        if label.is_null() {
            return String::new();
        }
        // SAFETY: `label` is the non-null label the live context returned.
        let name = unsafe { fdisk_label_get_name(label) };
        if name.is_null() {
            return String::new();
        }
        // SAFETY: libfdisk returns a NUL-terminated name that lives as long as `self.cxt`.
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    }

    fn slots(&self, disk: &str) -> Vec<Slot> {
        let mut table: *mut c_void = std::ptr::null_mut();
        // SAFETY: `self.cxt` is live and `table` starts null, so libfdisk writes an owned table or leaves it null.
        if unsafe { fdisk_get_partitions(self.cxt, &mut table) } < 0 || table.is_null() {
            return Vec::new();
        }
        let mut slots = Vec::new();
        for index in 0..unsafe { fdisk_table_get_nents(table) } {
            // SAFETY: `index` is below the entry count just read from the live table.
            let part = unsafe { fdisk_table_get_partition(table, index) };
            if part.is_null() {
                continue;
            }
            if unsafe { fdisk_partition_has_start(part) } == 0
                || unsafe { fdisk_partition_has_size(part) } == 0
                || unsafe { fdisk_partition_has_partno(part) } == 0
            {
                continue;
            }
            let number = unsafe { fdisk_partition_get_partno(part) };
            slots.push(Slot {
                node: partname(disk, number + 1),
                number: number + 1,
                start: unsafe { fdisk_partition_get_start(part) },
                sectors: unsafe { fdisk_partition_get_size(part) },
            });
        }
        // SAFETY: `table` is the table libfdisk returned and no reference to it is used after this.
        unsafe { fdisk_unref_table(table) };
        slots
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        if self.assigned {
            // SAFETY: `assigned` means `fdisk_assign_device` accepted this live context.
            unsafe { fdisk_deassign_device(self.cxt, 1) };
        }
        // SAFETY: `self.cxt` came from `fdisk_new_context` and no use of it follows this drop.
        unsafe { fdisk_unref_context(self.cxt) };
    }
}

pub(crate) fn partname(disk: &str, number: usize) -> String {
    let Ok(path) = CString::new(disk) else {
        return String::new();
    };
    // SAFETY: `path` is NUL-terminated and outlives the call, and libfdisk returns an allocated name or null.
    let name = unsafe { fdisk_partname(path.as_ptr(), number) };
    if name.is_null() {
        return String::new();
    }
    // SAFETY: `name` is the non-null NUL-terminated string libfdisk allocated.
    let node = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `name` came from `fdisk_partname`, which allocates with malloc, and this frees it once.
    unsafe { libc::free(name.cast()) };
    node
}

pub(crate) fn read_table(disk: &str) -> Result<DiskTable, String> {
    const ALIGN: u64 = 1024 * 1024;
    let context = Context::read_only(disk)?;
    let sector = unsafe { fdisk_get_sector_size(context.cxt) } as u64;
    let mut table = DiskTable {
        sector: match sector {
            0 => 512,
            size => size,
        },
        label: context.label(),
        first: unsafe { fdisk_get_first_lba(context.cxt) },
        last: unsafe { fdisk_get_last_lba(context.cxt) },
        slots: context.slots(disk),
    };
    // A non-GPT label keeps the 1 MiB start and the 34-sector tail that a future GPT needs.
    if table.label != "gpt" {
        table.first = ALIGN / table.sector;
        // SAFETY: `context` owns a live context for the whole function.
        table.last = unsafe { fdisk_get_nsectors(context.cxt) }.saturating_sub(34);
    }
    Ok(table)
}
