"""What a built VM asset is a function of.

`AssetLanes._build` shelled into `capsem-admin image build` for every profile
and every stage, every run, with no check of any kind. Four consecutive
qualifications of one release spent about twenty-five minutes each rebuilding
both architectures from sources none of them had touched -- the last three
changed only test files and a shell function.

The build cache does carry `assets/` between prefixes, and the lane ignored
it: the only thing consulting it is the `_when_missing` recovery path, which
answers a different question. The lane's own output tree is not carried at
all, so there was nothing to reuse even in principle.

Reuse needs an identity, and the identity has to be wider than it strictly
needs to be. Over-hashing costs a rebuild nobody notices. Under-hashing ships
a stale rootfs into a release, and the run that does it is green. So the roots
are declared in `config/gate.toml`, reviewable in one place, and a root that
does not exist is a typo rather than "nothing here" -- the failure that
quietly shrinks an identity to whatever happened to be present.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

from .config import GateConfig
from .errors import GateError


def roots(config: GateConfig) -> tuple[str, ...]:
    """Every checkout path a built asset can depend on."""
    return config.assets.identity_roots


def digest_of(root: Path, relatives: tuple[str, ...]) -> str:
    """One digest over the declared trees, stable across machines.

    Names are hashed alongside contents: a file that appears changes what the
    initrd packs, and content-only hashing would call that identical.
    """
    digest = hashlib.blake2b(digest_size=16)
    for relative in relatives:
        target = root / relative
        if not target.exists():
            raise GateError(
                f"asset identity root {relative!r} does not exist under {root}; "
                "a root that is missing reads as 'nothing here' and silently "
                "shrinks the identity to whatever happened to be present"
            )
        for path in sorted(_files(target)):
            digest.update(path.relative_to(root).as_posix().encode("utf-8"))
            digest.update(b"\0")
            digest.update(path.read_bytes() if path.is_file() else b"")
            digest.update(b"\0")
    return digest.hexdigest()


def _files(target: Path):
    if target.is_file():
        yield target
        return
    for path in target.rglob("*"):
        # `__pycache__` is regenerated per interpreter run and belongs to no
        # asset; hashing it would make every identity unique by accident.
        if path.is_file() and "__pycache__" not in path.parts:
            yield path


def lane_identity(config: GateConfig) -> str:
    """The digest a lane records beside its output and checks before rebuilding."""
    return digest_of(config.root, roots(config))
