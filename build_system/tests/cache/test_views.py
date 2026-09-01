"""Generated package and release views retain exact object receipts."""

from pathlib import Path

from capsem_builder.cache.models import CachePolicy, PruneMethod, StagePolicy
from capsem_builder.cache.objects import object_path
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.views import ReceiptLocation, ViewReceipt, canonicalize, copy_view


def test_named_view_is_hardlinked_and_receipted(tmp_path: Path) -> None:
    stage = StagePolicy(
        path=Path("objects"),
        warning_bytes=1,
        soft_bytes=2,
        hard_bytes=3,
        prune=PruneMethod.NONE,
        maximum_age_hours=1,
    )
    paths = CachePaths(
        repository_root=tmp_path,
        policy=CachePolicy(
            version=1, root=Path("cache"), minimum_free_bytes=1, stages={"objects": stage}
        ),
    )
    package = tmp_path / "Capsem.deb"
    package.write_bytes(b"package")

    receipt = canonicalize(paths, package)
    loaded = ViewReceipt.model_validate_json(
        package.with_name("Capsem.deb.object.json").read_text(encoding="utf-8")
    )

    assert loaded == receipt
    assert package.read_bytes() == b"package"
    assert package.stat().st_ino == object_path(paths, receipt.object).stat().st_ino

    staged = tmp_path / "release" / package.name
    staged_receipt = copy_view(paths, package, staged, receipt_location=ReceiptLocation.INVENTORY)

    assert staged_receipt.object == receipt.object
    assert staged.stat().st_ino == package.stat().st_ino
    assert not staged.with_name(f"{staged.name}.object.json").exists()
    assert (
        paths.stage("objects") / "receipts/views" / receipt.object.digest / f"{staged.name}.json"
    ).is_file()
