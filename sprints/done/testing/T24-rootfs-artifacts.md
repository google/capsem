# Sprint T24: Rootfs Artifacts
## Goal
Single source of truth for rootfs artifacts, validated in Dockerfile and build context

## Files
- src/capsem/builder/docker.py (modify)
- tests/test_rootfs_artifacts.py (new)

## Tasks
- [x] Define ROOTFS_REQUIRED_ARTIFACTS constant in docker.py (files + dirs)
- [x] prepare_build_context() uses constant to copy files
- [x] doctor.py imports constant for check_source_files()
- [x] validate.py imports constant for _validate_artifacts()
- [x] Template: add snapshots COPY line for explicit destination
- [x] test_rootfs_artifacts_in_dockerfile: render template, every constant entry has COPY/ADD line
- [x] test_rootfs_artifacts_in_build_context: call prepare_build_context() into temp dir, every entry exists
- [x] test_constant_used_everywhere: grep imports, no hardcoded lists
- [x] Marked rootfs, no VM needed

## Verification
pytest tests/test_rootfs_artifacts.py -m rootfs

## Depends On
T0
