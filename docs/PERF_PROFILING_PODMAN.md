# Perf Profiling in Podman Containers

## Status Update (2025-11-29)

**Perf profiling now works in this container!** The container has been launched with the necessary capabilities (`CAP_PERFMON` and `CAP_SYS_ADMIN`), so `make perfprofile` works without requiring `sudo`.

To verify perf is working:
```bash
# Quick test
perf stat ls

# Check capabilities
capsh --print | grep cap_perfmon
```

The Makefile has been updated to:
1. Remove the `sudo` requirement (no longer needed with expanded permissions)
2. Fix `rewind_bench` CLI arguments (`--sequential` → `-m sequential`)
3. Explicitly specify output file (`-o perf.data`) to prevent piping issues

## Historical Context: Problem Summary

When running `make perfprofile` in a Podman container, you'll encounter permission errors:

```
perf_event_open(..., PERF_FLAG_FD_CLOEXEC) failed with unexpected error 1 (Operation not permitted)
perf_event_open(..., 0) failed unexpectedly with error 1 (Operation not permitted)
Error:
No permission to enable cycles:P event.
```

This occurs even when:
- Running with `sudo` inside the container
- The `perf_event_paranoid` sysctl is set to `0` (permissive)
- The `/proc/sys` filesystem is read-only in the container

## Root Cause

The issue has two components:

1. **Missing Linux Capabilities**: Podman containers run with reduced capabilities by default. The `perf_event_open()` syscall requires the `CAP_PERFMON` capability (on kernel 5.8+) or `CAP_SYS_ADMIN` (older kernels).

2. **Read-Only /sys**: Even if we could modify `/proc/sys/kernel/perf_event_paranoid`, it's mounted read-only in the container:
   ```
   sysfs on /sys type sysfs (ro,nosuid,nodev,noexec,relatime,seclabel)
   ```

3. **Syscall Filtering**: Podman's default seccomp profile may restrict `perf_event_open()` syscalls for security.

## Solution: Podman Run Options

To enable perf profiling in a Podman container, you need to run it with additional privileges:

### Option 1: Add Specific Capabilities (Recommended)

```bash
podman run --cap-add=CAP_PERFMON --cap-add=CAP_SYS_ADMIN \
           --security-opt seccomp=unconfined \
           your-container-image
```

**What this does:**
- `--cap-add=CAP_PERFMON`: Grants performance monitoring capabilities (kernel 5.8+)
- `--cap-add=CAP_SYS_ADMIN`: Grants system admin capabilities (needed on older kernels, also helps with various perf features)
- `--security-opt seccomp=unconfined`: Disables seccomp filtering to allow perf_event_open syscalls

### Option 2: Privileged Mode (Less Secure, but Simple)

```bash
podman run --privileged your-container-image
```

**What this does:**
- Grants all capabilities to the container
- Disables security restrictions
- **WARNING**: Only use for development/profiling, not production

### Option 3: Mount Host's /proc/sys (Read/Write)

```bash
podman run --cap-add=CAP_PERFMON --cap-add=CAP_SYS_ADMIN \
           --security-opt seccomp=unconfined \
           -v /proc/sys/kernel:/host-sys:ro \
           your-container-image
```

Then inside the container, you can read the host's perf_event_paranoid:
```bash
cat /host-sys/perf_event_paranoid
```

### Option 4: Kernel Parameter Adjustment (Host System)

On the **host system** (not container), set perf_event_paranoid to -1:

```bash
# Temporary (until reboot)
sudo sysctl -w kernel.perf_event_paranoid=-1

# Permanent (survives reboot)
echo "kernel.perf_event_paranoid = -1" | sudo tee -a /etc/sysctl.d/99-perf.conf
sudo sysctl -p /etc/sysctl.d/99-perf.conf
```

**Paranoid levels:**
- `-1`: No restrictions (any user can profile)
- `0`: Disallow raw tracepoint access for unpriv
- `1`: Disallow CPU event access for unpriv
- `2`: Disallow kernel profiling for unpriv (default)
- `3`: Disallow all perf events for unpriv
- `4`: Disallow all perf events, even for root

Then run Podman with capabilities:
```bash
podman run --cap-add=CAP_PERFMON --cap-add=CAP_SYS_ADMIN \
           --security-opt seccomp=unconfined \
           your-container-image
```

## Complete Example for This Project

Assuming you're using Podman and want to profile the MTG Forge-rs project:

```bash
# On host system (optional, but recommended)
sudo sysctl -w kernel.perf_event_paranoid=-1

# Run container with profiling capabilities
podman run --cap-add=CAP_PERFMON \
           --cap-add=CAP_SYS_ADMIN \
           --security-opt seccomp=unconfined \
           -v $(pwd):/mtg-forge-rs-fedora:Z \
           -w /mtg-forge-rs-fedora \
           -it your-dev-container bash

# Inside container
make perfprofile
```

## Verification

To verify perf is working inside the container:

```bash
# Test basic perf stat (doesn't require recording)
perf stat ls

# Test perf record with a simple command
perf record -F 99 sleep 1
perf report

# Check capabilities
capsh --print | grep cap_perfmon
```

## Alternative: Profile on Host System

The simplest approach is to run profiling on the host system:

```bash
# Build in container
podman run ... make build-release

# Copy binary to host
podman cp container_name:/mtg-forge-rs-fedora/target/release/rewind_bench ./

# Profile on host
perf record -F 997 -g --call-graph dwarf ./rewind_bench -n 5000 --sequential
perf report
```

## Why This Matters

Performance profiling with `perf` provides:
- **CPU hotspots**: Identifies which functions consume the most CPU time
- **Call graphs**: Shows the complete call stack for expensive operations
- **Cache behavior**: Measures L1/L2/L3 cache miss rates
- **IPC (Instructions Per Cycle)**: Helps identify CPU pipeline stalls
- **Branch prediction**: Shows mispredicted branches

This is complementary to DHAT allocation profiling (`make dhatprofile`), which works fine in containers without special permissions.

## See Also

- `make dhatprofile` - Allocation profiling (works in containers without special permissions)
- `make profile` - Flamegraph profiling (requires cargo-flamegraph but may work with fewer privileges)
- `make heapprofile` - Heaptrack profiling (alternative allocation profiler)

## References

- [Linux perf_event_open man page](https://man7.org/linux/man-pages/man2/perf_event_open.2.html)
- [Podman run capabilities](https://docs.podman.io/en/latest/markdown/podman-run.1.html#cap-add-capability)
- [Linux capabilities](https://man7.org/linux/man-pages/man7/capabilities.7.html)
- [CAP_PERFMON documentation](https://lwn.net/Articles/812502/)
