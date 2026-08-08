# Sprint T15: Build Chain
## Goal
End-to-end build chain test: cargo build -> codesign -> pack-initrd -> manifest -> boot VM

## Files
- tests/capsem-build-chain/conftest.py
- tests/capsem-build-chain/test_cargo_build.py
- tests/capsem-build-chain/test_codesign.py
- tests/capsem-build-chain/test_pack_initrd.py
- tests/capsem-build-chain/test_manifest_regen.py
- tests/capsem-build-chain/test_full_chain.py

## Tasks
- [x] test_cargo_build: build all 4 daemon crates, verify binaries exist in target/debug/
- [x] test_codesign: sign all binaries, codesign --verify succeeds
- [x] test_pack_initrd: run pack equivalent, output valid gzip+cpio, binaries 555, correct arch
- [x] test_manifest_regen: after pack, manifest.json hashes match b3sum of actual files
- [x] test_full_chain: build -> sign -> pack -> manifest -> boot VM -> exec "echo works" -> delete
- [x] All marked build_chain

## Verification
pytest tests/capsem-build-chain/ -m build_chain passes

## Depends On
T14
