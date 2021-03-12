use kvm_bindings as kvmb;
use libc::{c_int, c_ulong, c_void};
use nix::sys::uio::{process_vm_readv, process_vm_writev, IoVec, RemoteIoVec};
use nix::unistd::Pid;
use simple_error::{bail, try_with};
use std::ffi::OsStr;
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::os::unix::prelude::RawFd;

mod cpus;
mod ioctls;
mod memslots;

use crate::cpu::Regs;
use crate::inject_syscall;
use crate::kvm::ioctls::KVM_CHECK_EXTENSION;
use crate::kvm::memslots::get_maps;
use crate::proc::{openpid, Mapping, PidHandle};
use crate::result::Result;

pub struct Tracee<'a> {
    hypervisor: &'a Hypervisor,
    proc: inject_syscall::Process,
}

/// Safe wrapper for unsafe inject_syscall::Process operations.
impl<'a> Tracee<'a> {
    fn vm_ioctl(&self, request: c_ulong, arg: c_ulong) -> Result<c_int> {
        self.proc.ioctl(self.hypervisor.vm_fd, request, arg)
    }
    //fn cpu_ioctl(&self, cpu: usize, request: c_ulong, arg: c_int) -> Result<c_int> {
    //    self.proc
    //        .ioctl(self.hypervisor.vcpus[cpu].fd_num, request, arg)
    //}

    // comment borrowed from vmm-sys-util
    /// Run an [`ioctl`](http://man7.org/linux/man-pages/man2/ioctl.2.html)
    /// with an immutable reference.
    ///
    /// # Arguments
    ///
    /// * `req`: a device-dependent request code.
    /// * `arg`: an immutable reference passed to ioctl.
    ///
    /// # Safety
    ///
    /// The caller should ensure to pass a valid file descriptor and have the
    /// return value checked. Also he may take care to use the correct argument type belonging to
    /// the request type.
    pub unsafe fn vm_ioctl_with_ref<T: Sized + Copy>(
        self,
        request: c_ulong,
        arg: &T,
    ) -> Result<c_int> {
        let struct_arg: *mut c_void =
            try_with!(self.mmap(size_of::<T>()), "cannot allocate memory");

        try_with!(
            self.hypervisor.write(struct_arg, arg),
            "cannot write ioctl arg struct to hv"
        );

        let ioeventfd: kvmb::kvm_ioeventfd = try_with!(self.hypervisor.read(struct_arg), "foobar");
        println!(
            "arg {:?}, {:?}, {:?}",
            ioeventfd.len, ioeventfd.addr, ioeventfd.fd
        );

        println!("arg_ptr {:?}", struct_arg);
        let ret = self.vm_ioctl(request, struct_arg as c_ulong);

        // TODO
        //try_with!(
        //self.munmap(struct_arg, size_of::<T>()),
        //"cannot munmap memory allocated for ioctl request"
        //);

        ret
    }

    /// Make the kernel allocate anonymous memory (anywhere he likes, not bound to a file
    /// descriptor). This is not fully POSIX compliant, but works on linux.
    ///
    /// length in bytes.
    /// returns void pointer to the allocated virtual memory address of the hypervisor.
    pub fn mmap(&self, length: libc::size_t) -> Result<*mut c_void> {
        let addr = libc::AT_NULL as *mut c_void; // make kernel choose location for us
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_SHARED | libc::MAP_ANONYMOUS;
        let fd = 0 as RawFd; // ignored because of MAP_ANONYMOUS
        let offset = 0 as libc::off_t;
        self.proc.mmap(addr, length, prot, flags, fd, offset)
    }

    pub fn munmap(&self, _addr: *mut c_void, _length: libc::size_t) -> Result<()> {
        // TODO
        unimplemented!()
    }

    pub fn check_extension(&self, cap: c_int) -> Result<c_int> {
        self.vm_ioctl(KVM_CHECK_EXTENSION(), cap as c_ulong)
    }

    pub fn pid(&self) -> Pid {
        self.hypervisor.pid
    }
    pub fn get_maps(&self) -> Result<Vec<Mapping>> {
        get_maps(self)
    }
    pub fn mappings(&self) -> &[Mapping] {
        self.hypervisor.mappings.as_slice()
    }

    pub fn get_regs(&self, vcpu: &VCPU) -> Result<Regs> {
        let regs = Regs {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbp: 0,
            rbx: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rax: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            orig_rax: 0,
            rip: 0,
            cs: 0,
            eflags: 0,
            rsp: 0,
            ss: 0,
            fs_base: 0,
            gs_base: 0,
            ds: 0,
            es: 0,
            fs: 0,
            gs: 0,
        };
        Ok(regs)
    }
}

pub unsafe fn any_as_bytes<T: Sized>(p: &T) -> &[u8] {
    std::slice::from_raw_parts((p as *const T) as *const u8, size_of::<T>())
}

pub struct VCPU {
    pub idx: usize,
    pub fd_num: RawFd,
}

pub struct Hypervisor {
    pub pid: Pid,
    pub vm_fd: RawFd,
    pub vcpus: Vec<VCPU>,
    pub mappings: Vec<Mapping>,
}

impl Hypervisor {
    pub fn attach(&self) -> Result<Tracee> {
        let proc = try_with!(
            inject_syscall::attach(self.pid),
            "cannot attach to hypervisor"
        );
        Ok(Tracee {
            hypervisor: self,
            proc,
        })
    }

    /// read from a virtual addr of the hypervisor
    pub fn read<T: Sized + Copy>(&self, addr: *const c_void) -> Result<T> {
        let len = size_of::<T>();
        let mut t_mem = MaybeUninit::<T>::uninit();
        let t_slice = unsafe { std::slice::from_raw_parts_mut(t_mem.as_mut_ptr() as *mut u8, len) };

        let local_iovec = vec![IoVec::from_mut_slice(t_slice)];
        let remote_iovec = vec![RemoteIoVec {
            base: addr as usize,
            len,
        }];

        let f = try_with!(
            process_vm_readv(self.pid, local_iovec.as_slice(), remote_iovec.as_slice()),
            "cannot read hypervisor memory"
        );
        if f != len {
            bail!(
                "process_vm_readv read {} bytes when {} were expected",
                f,
                len
            )
        }

        let t: T = unsafe { t_mem.assume_init() };
        Ok(t)
    }

    /// write to a virtual addr of the hypervisor
    pub fn write<T: Sized + Copy>(&self, addr: *mut c_void, val: &T) -> Result<()> {
        let len = size_of::<T>();
        // safe, because we won't need t_bytes for long
        let t_bytes = unsafe { any_as_bytes(val) };

        let local_iovec = vec![IoVec::from_slice(t_bytes)];
        let remote_iovec = vec![RemoteIoVec {
            base: addr as usize,
            len,
        }];

        let f = try_with!(
            process_vm_writev(self.pid, local_iovec.as_slice(), remote_iovec.as_slice()),
            "cannot write hypervisor memory"
        );
        if f != len {
            bail!(
                "process_vm_writev written {} bytes when {} were expected",
                f,
                len
            )
        }

        Ok(())
    }
}

fn find_vm_fd(handle: &PidHandle) -> Result<(Vec<RawFd>, Vec<VCPU>)> {
    let mut vm_fds: Vec<RawFd> = vec![];
    let mut vcpu_fds: Vec<VCPU> = vec![];
    let fds = try_with!(
        handle.fds(),
        "cannot lookup file descriptors of process {}",
        handle.pid
    );

    for fd in fds {
        let name = fd
            .path
            .file_name()
            .unwrap_or_else(|| OsStr::new(""))
            .to_str()
            .unwrap_or("");
        if name == "anon_inode:kvm-vm" {
            vm_fds.push(fd.fd_num)
        // i.e. anon_inode:kvm-vcpu:0
        } else if name.starts_with("anon_inode:kvm-vcpu:") {
            let parts = name.rsplitn(2, ':').collect::<Vec<_>>();
            assert!(parts.len() == 2);
            let idx = try_with!(
                parts[0].parse::<usize>(),
                "cannot parse number {}",
                parts[0]
            );
            vcpu_fds.push(VCPU {
                idx,
                fd_num: fd.fd_num,
            })
        }
    }
    let old_len = vcpu_fds.len();
    vcpu_fds.dedup_by_key(|vcpu| vcpu.idx);
    if old_len != vcpu_fds.len() {
        bail!("found multiple vcpus with same id, assume multiple VMs in same hypervisor. This is not supported yet")
    };

    Ok((vm_fds, vcpu_fds))
}

pub fn get_hypervisor(pid: Pid) -> Result<Hypervisor> {
    let handle = try_with!(openpid(pid), "cannot open handle in proc");

    let (vm_fds, vcpus) = try_with!(find_vm_fd(&handle), "failed to access kvm fds");
    let mappings = try_with!(handle.maps(), "cannot read process maps");
    if vm_fds.is_empty() {
        bail!("no VMs found");
    }
    if vm_fds.len() > 1 {
        bail!("multiple VMs found, this is not supported yet.");
    }
    if vcpus.is_empty() {
        bail!("found KVM instance but no VCPUs");
    }

    Ok(Hypervisor {
        pid,
        vm_fd: vm_fds[0],
        vcpus,
        mappings,
    })
}