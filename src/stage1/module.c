#include <linux/module.h>

#include "stage1.h"

#define MAX_STAGE2_ARGS 254
static int stage2_argc;
static char *stage2_argv[MAX_STAGE2_ARGS];

#define MAX_DEVICES 254
static int devices_num;
static char *devices[3];

// FIXME: Right now this is a kernel module in future, this should be replaced
// something to be injectable into VMs.
int init_module(void) {
  unsigned long long devs[3];
  size_t i;
  for (i = 0; i < devices_num; i++) {
    if (kstrtoull(devices[i], 10, &devs[i])) {
      printk("stage1: invalid mmio address: %s\n", devices[i]);
      return -EINVAL;
    }
    printk("stage1: addr: %llx\n", devs[i]);
  }

  return init_vmsh_stage1(devices_num, devs, stage2_argc, stage2_argv);
}

void cleanup_module(void) {
  cleanup_vmsh_stage1();
}

module_param_array(devices, charp, &devices_num, 0);
module_param_array(stage2_argv, charp, &stage2_argc, 0);

MODULE_AUTHOR("joerg@thalheim.io");
MODULE_DESCRIPTION("Mount block device and launch intial vmsh process");
MODULE_LICENSE("GPL");
