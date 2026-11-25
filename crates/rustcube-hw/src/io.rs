//! `Mem.io_read`/`io_write` take one function each; this routes address
//! ranges to the modules that claimed them and the rest to whatever the
//! host had installed before.
use rustcube_recomp::rt::Mem;
use rustcube_recomp::{global, global::Global};

pub type Read = fn(u32, u32) -> u32;
pub type Write = fn(u32, u32, u32);

struct Routes {
    reads: Vec<(u32, u32, Read)>,
    writes: Vec<(u32, u32, Write)>,
    read_rest: Read,
    write_rest: Write,
}

static ROUTES: Global<Routes> = global!(Routes {
    reads: Vec::new(),
    writes: Vec::new(),
    read_rest: |_, _| 0,
    write_rest: |_, _, _| {},
});

/// Bumped on every route change; each thread keeps a copy of the routes and
/// refreshes it when this moves. The GX pipe alone sends millions of writes a
/// frame through here, and taking the routes' lock for each one showed up as
/// ~8% of the game thread in a browser profile.
static VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct Local {
    version: u64,
    reads: Vec<(u32, u32, Read)>,
    writes: Vec<(u32, u32, Write)>,
    read_rest: Read,
    write_rest: Write,
}

thread_local! {
    static LOCAL: std::cell::RefCell<Local> = std::cell::RefCell::new(Local {
        version: 0,
        reads: Vec::new(),
        writes: Vec::new(),
        read_rest: |_, _| 0,
        write_rest: |_, _, _| {},
    });
}

fn with_local<R>(f: impl FnOnce(&Local) -> R) -> R {
    LOCAL.with(|l| {
        let now = VERSION.load(std::sync::atomic::Ordering::Acquire);
        if l.borrow().version != now {
            ROUTES.with_borrow(|r| {
                *l.borrow_mut() = Local {
                    version: now,
                    reads: r.reads.clone(),
                    writes: r.writes.clone(),
                    read_rest: r.read_rest,
                    write_rest: r.write_rest,
                };
            });
        }
        f(&l.borrow())
    })
}

fn dispatch_read(a: u32, size: u32) -> u32 {
    let f = with_local(|r| {
        r.reads
            .iter()
            .find(|(s, e, _)| (*s..*e).contains(&a))
            .map_or(r.read_rest, |x| x.2)
    });
    f(a, size)
}

fn dispatch_write(a: u32, size: u32, v: u32) {
    let f = with_local(|r| {
        r.writes
            .iter()
            .find(|(s, e, _)| (*s..*e).contains(&a))
            .map_or(r.write_rest, |x| x.2)
    });
    f(a, size, v)
}

/// Put the router in front of `m`'s hooks (once per `Mem`).
fn take_over(m: &mut Mem) {
    ROUTES.with_borrow_mut(|r| {
        if !std::ptr::fn_addr_eq(m.io_read, dispatch_read as Read) {
            r.read_rest = m.io_read;
            m.io_read = dispatch_read;
        }
        if !std::ptr::fn_addr_eq(m.io_write, dispatch_write as Write) {
            r.write_rest = m.io_write;
            m.io_write = dispatch_write;
        }
    });
    VERSION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

/// Reads of `start..end` go to `f`.
pub fn route_read(m: &mut Mem, start: u32, end: u32, f: Read) {
    take_over(m);
    ROUTES.with_borrow_mut(|r| r.reads.insert(0, (start, end, f)));
    VERSION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

/// Writes to `start..end` go to `f`.
pub fn route_write(m: &mut Mem, start: u32, end: u32, f: Write) {
    take_over(m);
    ROUTES.with_borrow_mut(|r| r.writes.insert(0, (start, end, f)));
    VERSION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

/// The bytes of a `size`-byte store of `v`, as they hit the bus.
pub fn bytes(size: u32, v: u32) -> Vec<u8> {
    match size {
        1 => vec![v as u8],
        2 => (v as u16).to_be_bytes().to_vec(),
        _ => v.to_be_bytes().to_vec(),
    }
}
