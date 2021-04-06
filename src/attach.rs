//mod device;

use crate::result::Result;
use simple_error::try_with;
use std::sync::Arc;

use crate::device::Device;
use crate::inspect::InspectOptions;
use crate::kvm;

pub fn attach(opts: &InspectOptions) -> Result<()> {
    println!("attaching");

    let vm = Arc::new(try_with!(
        kvm::hypervisor::get_hypervisor(opts.pid),
        "cannot get vms for process {}",
        opts.pid
    ));
    vm.stop()?;

    let device = try_with!(Device::new(&vm), "cannot create vm");
    vm.resume()?;
    device.create();
    device.create();
    println!("pause");
    nix::unistd::pause();
    Ok(())
}
