#!/usr/bin/env python3
"""KVM diagnostic probe for restricted/nested environments.

Run on a Cloudtop or any machine where Capsem boot fails with:
  KVM_CREATE_VCPU(0) failed: File exists (os error 17)

Tests each KVM ioctl individually to pinpoint the failure.

Usage: python3 scripts/kvm-diagnostic.py
"""
import ctypes
import fcntl
import os
import struct
import sys
import array

# KVM ioctl numbers (from linux/kvm.h)
KVMIO = 0xAE
KVM_GET_API_VERSION    = KVMIO << 8 | 0x00
KVM_CREATE_VM          = KVMIO << 8 | 0x01
KVM_CHECK_EXTENSION    = KVMIO << 8 | 0x03
KVM_GET_VCPU_MMAP_SIZE = KVMIO << 8 | 0x04

# VM ioctls
KVM_CREATE_VCPU         = KVMIO << 8 | 0x41
KVM_SET_USER_MEMORY_REGION = 0x4020AE46
KVM_CREATE_IRQCHIP      = KVMIO << 8 | 0x60
KVM_CREATE_PIT2         = 0x4040AE77
KVM_SET_TSS_ADDR        = KVMIO << 8 | 0xD7  # _IO
KVM_SET_IDENTITY_MAP_ADDR = 0x4008AE48
KVM_GET_SUPPORTED_CPUID = 0xC008AE05  # _IOWR

# Capabilities
KVM_CAP_IRQCHIP       = 0
KVM_CAP_NR_VCPUS      = 9
KVM_CAP_MAX_VCPUS     = 66
KVM_CAP_SPLIT_IRQCHIP = 121
KVM_CAP_NR_MEMSLOTS   = 10

PASS = "\033[32mOK\033[0m"
FAIL = "\033[31mFAIL\033[0m"
WARN = "\033[33mWARN\033[0m"
INFO = "\033[36mINFO\033[0m"


def check(label, fn):
    """Run fn(), print result."""
    try:
        result = fn()
        print(f"  [{PASS}] {label}: {result}")
        return result
    except OSError as e:
        print(f"  [{FAIL}] {label}: {e}")
        return None


def kvm_ioctl(fd, request, arg=0):
    """Raw ioctl, returns result or raises OSError."""
    ret = fcntl.ioctl(fd, request, arg)
    return ret


def main():
    print("=" * 60)
    print("Capsem KVM Diagnostic")
    print("=" * 60)
    print()

    # -- Phase 1: /dev/kvm existence and basic info -----------------------
    print("[Phase 1] /dev/kvm basics")
    if not os.path.exists("/dev/kvm"):
        print(f"  [{FAIL}] /dev/kvm does not exist")
        sys.exit(1)

    stat = os.stat("/dev/kvm")
    print(f"  [{INFO}] /dev/kvm mode: {oct(stat.st_mode)}")
    print(f"  [{INFO}] /dev/kvm uid:gid: {stat.st_uid}:{stat.st_gid}")

    try:
        kvm = os.open("/dev/kvm", os.O_RDWR | os.O_CLOEXEC)
    except OSError as e:
        print(f"  [{FAIL}] open(/dev/kvm): {e}")
        sys.exit(1)
    print(f"  [{PASS}] open(/dev/kvm): fd={kvm}")

    api_ver = check("KVM_GET_API_VERSION",
                     lambda: fcntl.ioctl(kvm, KVM_GET_API_VERSION, 0))

    mmap_size = check("KVM_GET_VCPU_MMAP_SIZE",
                      lambda: fcntl.ioctl(kvm, KVM_GET_VCPU_MMAP_SIZE, 0))

    # -- Phase 2: capabilities -------------------------------------------
    print()
    print("[Phase 2] KVM capabilities")
    caps = {
        "KVM_CAP_IRQCHIP": KVM_CAP_IRQCHIP,
        "KVM_CAP_NR_VCPUS": KVM_CAP_NR_VCPUS,
        "KVM_CAP_MAX_VCPUS": KVM_CAP_MAX_VCPUS,
        "KVM_CAP_SPLIT_IRQCHIP": KVM_CAP_SPLIT_IRQCHIP,
        "KVM_CAP_NR_MEMSLOTS": KVM_CAP_NR_MEMSLOTS,
    }
    for name, cap in caps.items():
        check(name, lambda c=cap: fcntl.ioctl(kvm, KVM_CHECK_EXTENSION, c))

    # -- Phase 3: test sequence matching Capsem boot ----------------------
    print()
    print("[Phase 3] Capsem boot sequence (irqchip THEN vcpu)")
    print("  This matches the current Capsem code path.")
    vm1 = check("KVM_CREATE_VM", lambda: fcntl.ioctl(kvm, KVM_CREATE_VM, 0))
    if vm1 is None:
        print(f"  [{FAIL}] Cannot proceed without VM fd")
        os.close(kvm)
        sys.exit(1)

    check("KVM_SET_TSS_ADDR(0xFFFBD000)",
          lambda: fcntl.ioctl(vm1, KVM_SET_TSS_ADDR, struct.unpack("i", struct.pack("I", 0xFFFBD000))[0]))

    check("KVM_SET_IDENTITY_MAP_ADDR(0xFFFBC000)",
          lambda: fcntl.ioctl(vm1, KVM_SET_IDENTITY_MAP_ADDR,
                              struct.pack("Q", 0xFFFBC000)))

    irqchip_ok = check("KVM_CREATE_IRQCHIP",
                       lambda: fcntl.ioctl(vm1, KVM_CREATE_IRQCHIP, 0))

    check("KVM_CREATE_PIT2",
          lambda: fcntl.ioctl(vm1, KVM_CREATE_PIT2, b"\x00" * 64))

    # CPUID probe (same as CI)
    buf = array.array("b", b"\x00" * 8200)
    struct.pack_into("I", buf, 0, 256)  # nent = 256
    check("KVM_GET_SUPPORTED_CPUID",
          lambda: fcntl.ioctl(vm1, KVM_GET_SUPPORTED_CPUID, buf, True))

    vcpu0_result = check("KVM_CREATE_VCPU(0)",
                         lambda: fcntl.ioctl(vm1, KVM_CREATE_VCPU, 0))

    if vcpu0_result is not None:
        print(f"\n  [{PASS}] Boot sequence works! VCPU 0 created (fd={vcpu0_result})")
        os.close(vcpu0_result)
    else:
        print(f"\n  [{FAIL}] VCPU 0 creation failed -- this is the Capsem bug")

    os.close(vm1)

    # -- Phase 4: test without irqchip -----------------------------------
    print()
    print("[Phase 4] No irqchip (vcpu ONLY)")
    print("  Tests if vcpu works when irqchip is NOT created.")
    vm2 = check("KVM_CREATE_VM",
                lambda: fcntl.ioctl(kvm, KVM_CREATE_VM, 0))
    if vm2 is not None:
        vcpu_no_irq = check("KVM_CREATE_VCPU(0) [no irqchip]",
                            lambda: fcntl.ioctl(vm2, KVM_CREATE_VCPU, 0))
        if vcpu_no_irq is not None:
            print(f"  [{INFO}] VCPU works without irqchip -- irqchip causes the problem")
            os.close(vcpu_no_irq)
        else:
            print(f"  [{INFO}] VCPU fails even without irqchip -- fundamental KVM restriction")
        os.close(vm2)

    # -- Phase 5: test vcpu BEFORE irqchip --------------------------------
    print()
    print("[Phase 5] Reversed order (vcpu THEN irqchip)")
    print("  Tests if creating vcpu before irqchip avoids the EEXIST.")
    print("  NOTE: standard KVM rejects this (IRQCHIP returns EINVAL if vcpus exist).")
    vm3 = check("KVM_CREATE_VM",
                lambda: fcntl.ioctl(kvm, KVM_CREATE_VM, 0))
    if vm3 is not None:
        vcpu_first = check("KVM_CREATE_VCPU(0) [before irqchip]",
                           lambda: fcntl.ioctl(vm3, KVM_CREATE_VCPU, 0))
        if vcpu_first is not None:
            os.close(vcpu_first)
            check("KVM_SET_TSS_ADDR(0xFFFBD000)",
                  lambda: fcntl.ioctl(vm3, KVM_SET_TSS_ADDR, struct.unpack("i", struct.pack("I", 0xFFFBD000))[0]))
            check("KVM_SET_IDENTITY_MAP_ADDR(0xFFFBC000)",
                  lambda: fcntl.ioctl(vm3, KVM_SET_IDENTITY_MAP_ADDR,
                                      struct.pack("Q", 0xFFFBC000)))
            irq_after = check("KVM_CREATE_IRQCHIP [after vcpu]",
                              lambda: fcntl.ioctl(vm3, KVM_CREATE_IRQCHIP, 0))
            if irq_after is not None:
                print(f"  [{INFO}] Reversed order works! This KVM allows vcpu-first.")
            else:
                print(f"  [{INFO}] Reversed order blocked: IRQCHIP requires no vcpus (standard kernel behavior).")
        os.close(vm3)

    # -- Phase 6: split irqchip ------------------------------------------
    print()
    print("[Phase 6] Split IRQCHIP mode")
    print("  Tests KVM_CAP_SPLIT_IRQCHIP as an alternative to full IRQCHIP.")
    vm4 = check("KVM_CREATE_VM",
                lambda: fcntl.ioctl(kvm, KVM_CREATE_VM, 0))
    if vm4 is not None:
        # KVM_ENABLE_CAP for split irqchip
        # struct kvm_enable_cap { u32 cap, u32 flags, u64 args[4] }
        # cap = KVM_CAP_SPLIT_IRQCHIP (121), args[0] = 24 (num IOAPIC pins)
        enable_cap_ioctl = 0x4068AEA3  # KVM_ENABLE_CAP
        cap_buf = struct.pack("II QQQQ", KVM_CAP_SPLIT_IRQCHIP, 0, 24, 0, 0, 0)
        split_ok = check("KVM_ENABLE_CAP(SPLIT_IRQCHIP, pins=24)",
                         lambda: fcntl.ioctl(vm4, enable_cap_ioctl, cap_buf))
        if split_ok is not None:
            vcpu_split = check("KVM_CREATE_VCPU(0) [split irqchip]",
                               lambda: fcntl.ioctl(vm4, KVM_CREATE_VCPU, 0))
            if vcpu_split is not None:
                print(f"  [{INFO}] Split IRQCHIP + VCPU works! Possible workaround path.")
                os.close(vcpu_split)
            else:
                print(f"  [{INFO}] Split IRQCHIP doesn't help -- VCPU still fails.")
        else:
            print(f"  [{INFO}] Split IRQCHIP not supported on this KVM.")
        os.close(vm4)

    # -- Phase 7: kernel info -------------------------------------------
    print()
    print("[Phase 7] Environment info")
    uname = os.uname()
    print(f"  [{INFO}] kernel: {uname.release}")
    print(f"  [{INFO}] machine: {uname.machine}")
    print(f"  [{INFO}] hostname: {uname.nodename}")

    # Check if nested
    nested_path = "/sys/module/kvm_intel/parameters/nested"
    if not os.path.exists(nested_path):
        nested_path = "/sys/module/kvm_amd/parameters/nested"
    if os.path.exists(nested_path):
        with open(nested_path) as f:
            nested = f.read().strip()
        print(f"  [{INFO}] nested KVM: {nested}")
    else:
        print(f"  [{WARN}] nested KVM parameter not found")

    # Check KVM module
    try:
        with open("/proc/modules") as f:
            for line in f:
                if line.startswith("kvm"):
                    mod_name = line.split()[0]
                    print(f"  [{INFO}] module: {mod_name}")
    except OSError:
        pass

    os.close(kvm)

    # -- Summary ---------------------------------------------------------
    print()
    print("=" * 60)
    print("Summary")
    print("=" * 60)
    if vcpu0_result is not None:
        print("Standard boot sequence works. The issue may be transient")
        print("or environment-specific (other process holding KVM state).")
    else:
        print("KVM_CREATE_VCPU(0) fails after IRQCHIP creation.")
        print("Check phases 4-6 above for possible workarounds.")
    print()
    print("Please share this output with the Capsem team.")


if __name__ == "__main__":
    main()
