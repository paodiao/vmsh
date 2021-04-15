use kvm_bindings as kvmb;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use simple_error::bail;
use simple_error::try_with;
use std::fmt;

use crate::cpu::{self, Regs};
use crate::kvm::hypervisor;
use crate::kvm::ioctls;
use crate::kvm::memslots::get_vcpu_maps;
use crate::proc::Mapping;
use crate::ptrace;
use crate::result::Result;

type MmioRwRaw = kvmb::kvm_run__bindgen_ty_1__bindgen_ty_6;

pub struct MmioRw {
    /// address in the guest physical memory
    pub addr: u64,
    pub is_write: bool,
    data: [u8; 8],
    len: usize,
}

impl MmioRw {
    pub fn new(raw: &MmioRwRaw) -> MmioRw {
        // should we sanity check len here in order to not crash on out of bounds?
        MmioRw {
            addr: raw.phys_addr,
            is_write: raw.is_write != 0,
            data: raw.data,
            len: raw.len as usize,
        }
    }

    pub fn from(kvm_run: &kvmb::kvm_run) -> Option<MmioRw> {
        match kvm_run.exit_reason {
            kvmb::KVM_EXIT_MMIO => {
                // Safe because the exit_reason (which comes from the kernel) told us which
                // union field to use.
                let mmio: &MmioRwRaw = unsafe { &kvm_run.__bindgen_anon_1.mmio };
                Some(MmioRw::new(&mmio))
            }
            _ => None,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

impl fmt::Display for MmioRw {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Write strictly the first element into the supplied output
        // stream: `f`. Returns `fmt::Result` which indicates whether the
        // operation succeeded or failed. Note that `write!` uses syntax which
        // is very similar to `println!`.
        if self.is_write {
            write!(
                f,
                "MmioRw{{ write {:?} to guest phys @ 0x{:x} }}",
                self.data(),
                self.addr
            )
        } else {
            write!(
                f,
                "MmioRw{{ read {}b from guest phys @ 0x{:x} }}",
                self.len, self.addr
            )
        }
    }
}

/// Contains the state of the thread running a vcpu.
/// TODO in theory vcpus could change threads which they are run on
struct Thread {
    ptthread: ptrace::Thread,
    vcpu_map: Mapping,
    is_running: bool,
    in_syscall: bool,
}

impl Thread {
    pub fn new(ptthread: ptrace::Thread, vcpu_map: Mapping) -> Thread {
        Thread {
            ptthread,
            is_running: false,
            in_syscall: false, // ptrace (in practice) never attaches to a process while it is in a syscall
            vcpu_map,
        }
    }

    pub fn toggle_in_syscall(&mut self) {
        self.in_syscall = !self.in_syscall;
    }
}

/// TODO respect and handle newly spawned threads as well
pub struct KvmRunWrapper {
    process_idx: usize,
    threads: Vec<Thread>,
}

impl KvmRunWrapper {
    pub fn attach(pid: Pid, vcpu_maps: &Vec<Mapping>) -> Result<KvmRunWrapper> {
        //let threads = vec![try_with!(ptrace::attach(pid), "foo")];
        //let process_idx = 0;
        let (threads, process_idx) = try_with!(
            ptrace::attach_all_threads(pid),
            "cannot attach KvmRunWrapper to all threads of {} via ptrace",
            pid
        );
        let threads: Vec<Thread> = threads
            .into_iter()
            .map(|t| {
                let vcpu_map = vcpu_maps[0].clone(); // TODO support more than 1 cpu and respect remaps
                Thread::new(t, vcpu_map)
            })
            .collect();

        for t in &threads {
            let maps = get_vcpu_maps(t.ptthread.tid)?;
            println!("thread {} vcpu0 map: {:?}", t.ptthread.tid, maps[0]);
            assert_eq!(vcpu_maps[0].start, maps[0].start);
        }
        Ok(KvmRunWrapper {
            process_idx,
            threads,
        })
    }

    pub fn cont(&self) -> Result<()> {
        for thread in &self.threads {
            thread.ptthread.cont(None)?;
        }
        Ok(())
    }

    fn main_thread(&self) -> &Thread {
        &self.threads[self.process_idx]
    }

    fn main_thread_mut(&mut self) -> &mut Thread {
        &mut self.threads[self.process_idx]
    }

    // -> Err if third qemu thread terminates
    pub fn wait_for_ioctl(&mut self) -> Result<()> {
        //println!("syscall");
        for thread in &mut self.threads {
            if !thread.is_running {
                thread.ptthread.syscall()?;
                thread.is_running = true;
            }
        }
        //println!("syscall {}", self.threads[0].tid);
        //if !self.main_thread().is_running {
        //    try_with!(self.main_thread().ptthread.syscall(), "fii");
        //    self.main_thread_mut().is_running = true;
        //}

        //
        // Further options to waitpid on many Ps at the same time:
        //
        // - waitpid(WNOHANG): async waitpid, busy polling
        //
        // - linux waitid() P_PIDFD pidfd_open(): maybe (e)poll() on this fd? dunno
        //
        // - setpgid(): waitpid on the pgid. Grouping could destroy existing Hypervisor groups and
        //   requires all group members to be in the same session (whatever that means). Also if
        //   the group owner (pid==pgid) dies, the enire group orphans (will it be killed as
        //   zombies?)
        //   => sounds a bit dangerous, doesn't it?

        // use linux default flag of __WALL: wait for main_thread and all kinds of children
        // to wait for all children, use -gid
        //println!("wait {}", self.threads[0].tid);
        //println!("waitpid");
        let status = self.waitpid_busy()?;
        //let status = try_with!(
        //waitpid(Pid::from_raw(-self.main_thread().tid.as_raw()), None),
        //"cannot wait for ioctl syscall"
        //);
        if let Some(mmio) = self.process_status(status)? {
            println!("kvm exit: {}", mmio);
        }

        Ok(())
    }

    fn waitpid_busy(&mut self) -> Result<WaitStatus> {
        loop {
            for thread in &mut self.threads {
                let status = try_with!(
                    waitpid(
                        thread.ptthread.tid,
                        Some(nix::sys::wait::WaitPidFlag::WNOHANG)
                    ),
                    "cannot wait for ioctl syscall"
                );
                if WaitStatus::StillAlive != status {
                    //println!("waipid: {}", thread.ptthread.tid);
                    thread.is_running = false;
                    return Ok(status);
                }
            }
        }
    }

    fn process_status(&mut self, status: WaitStatus) -> Result<Option<MmioRw>> {
        match status {
            WaitStatus::PtraceEvent(_, _, _) => {
                bail!("got unexpected ptrace event")
            }
            WaitStatus::PtraceSyscall(_) => {
                bail!("got unexpected ptrace syscall event")
            }
            WaitStatus::StillAlive => {
                bail!("got unexpected still-alive waitpid() event")
            }
            WaitStatus::Continued(_) => {
                println!("WaitStatus::Continued");
            } // noop
            //WaitStatus::Stopped(_, Signal::SIGTRAP) => {
            //let regs =
            //try_with!(self.main_thread().getregs(), "cannot syscall results");
            //println!("syscall: eax {:x} ebx {:x}", regs.rax, regs.rbx);

            //return Ok(());
            //}
            WaitStatus::Stopped(pid, signal) => {
                let thread: &mut Thread = match self
                    .threads
                    .iter_mut()
                    .find(|thread| thread.ptthread.tid == pid)
                {
                    Some(t) => t,
                    None => bail!("received stop for unkown process: {}", pid),
                };

                let regs = try_with!(thread.ptthread.getregs(), "cannot syscall results");
                let (syscall_nr, ioctl_fd, ioctl_request) = regs.get_syscall_params();
                if syscall_nr == libc::SYS_ioctl as u64 {
                    // SYS_ioctl = 16
                } else {
                    return Ok(None);
                }
                // TODO check vcpufd and save a mapping for active syscalls from thread to cpu to
                // support multiple cpus
                thread.toggle_in_syscall();
                if ioctl_request == ioctls::KVM_RUN() {
                    // KVM_RUN = 0xae80 = ioctl_io_nr!(KVM_RUN, KVMIO, 0x80)
                    if thread.in_syscall {
                        println!("kvm-run enter");
                    } else {
                        println!("kvm-run exit.");
                    }
                    let map_ptr = thread.vcpu_map.start as *const kvm_bindings::kvm_run;
                    let kvm_run: kvm_bindings::kvm_run =
                        hypervisor::process_read(pid, map_ptr as *const libc::c_void)?;

                    let mmio = MmioRw::from(&kvm_run);
                    if mmio.is_some() {
                        println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!! EXIT_MMIO");
                    }

                    return Ok(mmio);
                } else {
                    return Ok(None);
                }
                println!("ioctl(fd = {}, request = 0x{:x})", ioctl_fd, ioctl_request);
                //println!("process {} was stopped by signal: {}", pid, signal);
                // println!(
                //     "syscall: eax {:x} ebx {:x} cs {:x} rip {:x}",
                //     regs.rax, regs.rbx, regs.cs, regs.rip
                // );
                let syscall_info = try_with!(thread.ptthread.syscall_info(), "cannot syscall info");
                println!("syscall info op: {:?}", syscall_info.op);
            }
            WaitStatus::Exited(_, status) => bail!("process exited with: {}", status),
            WaitStatus::Signaled(_, signal, _) => {
                bail!("process was stopped by signal: {}", signal)
            }
        }
        Ok(None)
    }

    // fn parse_kvm_run(kvm_run: &kvmb::kvm_run) -> Result<&MmioRW> {
    //     match kvm_run.exit_reason {
    //         // from https://github.com/rust-vmm/kvm-ioctls via MIT license
    //         KVM_EXIT_MMIO => {
    //             // Safe because the exit_reason (which comes from the kernel) told us which
    //             // union field to use.
    //             let mmio: &MmioRW = unsafe { &mut kvm_run.__bindgen_anon_1.mmio };
    //             let addr = mmio.phys_addr;
    //             let len = mmio.len as usize;
    //             let data_slice = &mut mmio.data[..len];
    //             if mmio.is_write != 0 {
    //                 Ok(VcpuExit::MmioWrite(addr, data_slice))
    //             } else {
    //                 Ok(VcpuExit::MmioRead(addr, data_slice))
    //             }
    //         }
    //     }
    //     Ok(())
    // }

    fn check_siginfo(&self, thread: &Thread) -> Result<()> {
        let siginfo = try_with!(
            nix::sys::ptrace::getsiginfo(thread.ptthread.tid),
            "cannot getsiginfo"
        );
        if (siginfo.si_code == libc::SIGTRAP) || (siginfo.si_code == (libc::SIGTRAP | 0x80)) {
            println!("siginfo.si_code true: 0x{:x}", siginfo.si_code);
            return Ok(());
        } else {
            println!("siginfo.si_code false: 0x{:x}", siginfo.si_code);
            //try_with!(nix::sys::ptrace::syscall(self.main_thread().tid, None), "cannot ptrace::syscall");
        }
        Ok(())
    }
}
