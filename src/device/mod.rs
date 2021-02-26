
mod virtio;

use vm_virtio::device::status::RESET;
use vm_device::resources::ResourceConstraint;
use vm_device::device_manager::IoManager;
use vm_device::bus::{MmioAddress, MmioRange};
use vm_memory::{GuestMemoryMmap, GuestRegionMmap, FileOffset};
use vm_memory::guest_memory::GuestAddress;
use vm_memory::mmap::MmapRegion;
use event_manager::{EventManager, MutEventSubscriber};
use std::sync::{Arc, Mutex};
use simple_error::{try_with, simple_error, SimpleError};
use std::path::PathBuf;

use crate::kvm::Hypervisor;
use crate::device::virtio::block::{self, BlockArgs};
use crate::device::virtio::{CommonArgs, MmioConfig};
use crate::proc::Mapping;
use crate::result::Result;

// Where BIOS/VGA magic would live on a real PC.
const EBDA_START: u64 = 0x9fc00;
const FIRST_ADDR_PAST_32BITS: u64 = 1 << 32;
const MEM_32BIT_GAP_SIZE: u64 = 768 << 20;
/// The start of the memory area reserved for MMIO devices.
pub const MMIO_MEM_START: u64 = FIRST_ADDR_PAST_32BITS - MEM_32BIT_GAP_SIZE;

type Block = block::Block<Arc<GuestMemoryMmap>>;

fn convert(mappings: &Vec<Mapping>) -> GuestMemoryMmap {
    let mut regions: Vec<Arc<GuestRegionMmap>> = vec!{};

    for mapping in mappings {
        let file = std::fs::File::open(&mapping.pathname).expect("could not open file"); // TODO formatted

        let file_offset = FileOffset::new(file, mapping.offset);
        // TODO i think we need Some(file_offset) in mmap_region
        // TODO need reason for why this is safe. ("a smart human wrote it")
        let mmap_region = unsafe { 
            MmapRegion::build_raw(
                mapping.start as *mut u8,
                (mapping.end - mapping.start) as usize,
                mapping.prot_flags.bits(),
                mapping.map_flags.bits()
            )
        }.expect("cannot instanciate MmapRegion");

        let guest_region_mmap = GuestRegionMmap::new(
            mmap_region,
            GuestAddress(mapping.phys_addr),
        ).expect("GuestRegionMmap error");

        regions.push(Arc::new(guest_region_mmap));
    }

    GuestMemoryMmap::from_arc_regions(regions).expect("GuestMemoryMmap error")
}

pub struct Device { 
    vmm: Arc<Hypervisor>,
    blkdev: Arc<Mutex<Block>>,
}

impl Device {
    pub fn new(vmm: &Arc<Hypervisor>) -> Result<Device> {

        let mem: Arc<GuestMemoryMmap> = Arc::new(convert(&vmm.mappings));

        let range = MmioRange::new(MmioAddress(MMIO_MEM_START), 0x1000).unwrap();
        let mmio_cfg = MmioConfig { range, gsi: 5 };

        // TODO is there more we have to do with this mgr?
        let device_manager = Arc::new(Mutex::new(IoManager::new())); 
        let mut guard = device_manager.lock().unwrap();

        let mut event_manager = try_with!(
            EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new(),
            "cannot create event manager");
        // TODO add subscriber (wrapped_exit_handler) and stuff?

        let common = CommonArgs {
            mem,
            vmm: vmm.clone(),
            event_mgr: &mut event_manager,
            mmio_mgr: guard,
            mmio_cfg,
        };

        let args = BlockArgs {
            common,
            file_path: PathBuf::from("/tmp/foobar"),
            read_only: false,
            root_device: true,
            advertise_flush: true,
        };

        let blkdev: Arc<Mutex<Block>> = Block::new(args).expect("cannot create block device");

        Ok(Device {
            vmm: vmm.clone(),
            blkdev,
        })

    }

    pub fn create(&self) {
        let a = RESET;
        let b = ResourceConstraint::new_pio(1);
        println!("create device");


    }
}
