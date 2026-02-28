"""Host-runnable unit tests for capsem-bench.

Tests helper functions, table rendering (Rich markup safety), and benchmark
logic that doesn't require the VM. Run with: pytest images/test_capsem_bench.py
"""

import importlib.machinery
import importlib.util
import json
import os
import stat
import sys
import tempfile
import types

import pytest


# ---------------------------------------------------------------------------
# Import capsem-bench as a module (it has no .py extension)
# ---------------------------------------------------------------------------

def _import_capsem_bench():
    bench_path = os.path.join(os.path.dirname(__file__), "capsem-bench")
    loader = importlib.machinery.SourceFileLoader("capsem_bench", bench_path)
    spec = importlib.util.spec_from_loader("capsem_bench", loader,
                                           origin=bench_path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


bench = _import_capsem_bench()


# ---------------------------------------------------------------------------
# Helper function tests
# ---------------------------------------------------------------------------

class TestPercentile:
    def test_empty(self):
        assert bench.percentile([], 50) == 0.0

    def test_single(self):
        assert bench.percentile([42.0], 50) == 42.0

    def test_median_odd(self):
        assert bench.percentile([1.0, 2.0, 3.0], 50) == 2.0

    def test_median_even(self):
        result = bench.percentile([1.0, 2.0, 3.0, 4.0], 50)
        assert result == pytest.approx(2.5)

    def test_p0(self):
        assert bench.percentile([10.0, 20.0, 30.0], 0) == 10.0

    def test_p100(self):
        assert bench.percentile([10.0, 20.0, 30.0], 100) == 30.0


class TestFmtBytes:
    def test_bytes(self):
        assert bench.fmt_bytes(512) == "512 B"

    def test_kilobytes(self):
        assert bench.fmt_bytes(2048) == "2.0 KB"

    def test_megabytes(self):
        assert bench.fmt_bytes(5 * 1024 * 1024) == "5.0 MB"

    def test_gigabytes(self):
        assert bench.fmt_bytes(3 * 1024 ** 3) == "3.0 GB"

    def test_zero(self):
        assert bench.fmt_bytes(0) == "0 B"


class TestThroughputMbps:
    def test_normal(self):
        # 1 MB in 1 second = 1.0 MB/s
        assert bench.throughput_mbps(1024 * 1024, 1.0) == 1.0

    def test_zero_duration(self):
        assert bench.throughput_mbps(1000, 0.0) == 0.0

    def test_negative_duration(self):
        assert bench.throughput_mbps(1000, -1.0) == 0.0


# ---------------------------------------------------------------------------
# Rich table rendering -- markup safety
# ---------------------------------------------------------------------------

class TestRichTableRendering:
    """Verify tables render without MarkupError for paths containing brackets."""

    def _render_table(self, table):
        """Render a table to a string, raising on any Rich error."""
        from io import StringIO
        from rich.console import Console
        buf = StringIO()
        c = Console(file=buf, width=200, force_terminal=False)
        c.print(table)
        return buf.getvalue()

    def _render_flat(self, table):
        """Render and collapse whitespace for wrap-safe assertions."""
        return " ".join(self._render_table(table).split())

    def test_disk_bench_title_with_slash_root(self):
        """The title '[/root, 256 MB]' must not crash Rich markup parser."""
        from rich.text import Text
        from rich.table import Table
        table = Table(title=Text("Scratch Disk I/O  [/root, 256 MB]"))
        table.add_column("Test")
        table.add_column("Value")
        table.add_row("Seq write", "100 MB/s")
        output = self._render_table(table)
        assert "/root" in output
        assert "256 MB" in output

    def test_http_bench_title_with_url(self):
        """The title with a URL containing slashes must not crash."""
        from rich.text import Text
        from rich.table import Table
        table = Table(title=Text("HTTP Benchmark  [https://www.google.com/]"))
        table.add_column("Metric")
        table.add_column("Value")
        table.add_row("Requests/sec", "42")
        output = self._render_table(table)
        assert "google.com" in output

    def test_startup_bench_title_with_brackets(self):
        """The title with '[3 runs each]' must not crash."""
        from rich.text import Text
        from rich.table import Table
        table = Table(title=Text("CLI Cold Start Latency  [3 runs each]"))
        table.add_column("Command")
        table.add_column("Min")
        table.add_row("python3", "50 ms")
        output = self._render_flat(table)
        assert "3 runs each" in output

    def test_markup_injection_in_directory_path(self):
        """Adversarial: directory path with Rich markup tags."""
        from rich.text import Text
        from rich.table import Table
        evil_path = "/tmp/[bold red]evil[/bold red]"
        table = Table(title=Text(f"Scratch Disk I/O  [{evil_path}, 64 MB]"))
        table.add_column("Test")
        table.add_row("test")
        output = self._render_flat(table)
        # Text() renders literally (not interpreted as markup styling).
        # Word-wrap may split "[bold" across lines, so check for "evil"
        # and "red]" which prove the brackets survived as literal text.
        assert "red]evil" in output
        assert "/tmp/" in output


# ---------------------------------------------------------------------------
# Rootfs file scanning
# ---------------------------------------------------------------------------

class TestFindLargestFile:
    def test_finds_largest(self, tmp_path):
        small = tmp_path / "small.bin"
        small.write_bytes(b"x" * 100)
        large = tmp_path / "large.bin"
        large.write_bytes(b"x" * 10000)
        medium = tmp_path / "medium.bin"
        medium.write_bytes(b"x" * 5000)

        path, size = bench.find_largest_file([str(tmp_path)])
        assert path == str(large)
        assert size == 10000

    def test_empty_directory(self, tmp_path):
        path, size = bench.find_largest_file([str(tmp_path)])
        assert path is None
        assert size == 0

    def test_nonexistent_directory(self):
        path, size = bench.find_largest_file(["/nonexistent/dir/xyz"])
        assert path is None
        assert size == 0

    def test_skips_symlinks(self, tmp_path):
        real = tmp_path / "real.bin"
        real.write_bytes(b"x" * 100)
        link = tmp_path / "link.bin"
        link.symlink_to(real)

        path, size = bench.find_largest_file([str(tmp_path)])
        assert path == str(real)

    def test_nested_directories(self, tmp_path):
        nested = tmp_path / "a" / "b" / "c"
        nested.mkdir(parents=True)
        deep = nested / "deep.bin"
        deep.write_bytes(b"x" * 50000)
        shallow = tmp_path / "shallow.bin"
        shallow.write_bytes(b"x" * 100)

        path, size = bench.find_largest_file([str(tmp_path)])
        assert path == str(deep)
        assert size == 50000


class TestCollectRootfsFiles:
    def test_collects_above_min_size(self, tmp_path):
        big = tmp_path / "big.bin"
        big.write_bytes(b"x" * 8192)
        tiny = tmp_path / "tiny.bin"
        tiny.write_bytes(b"x" * 10)

        files = bench.collect_rootfs_files([str(tmp_path)], min_size=4096)
        paths = [f[0] for f in files]
        assert str(big) in paths
        assert str(tiny) not in paths

    def test_empty_dir(self, tmp_path):
        files = bench.collect_rootfs_files([str(tmp_path)])
        assert files == []

    def test_skips_symlinks(self, tmp_path):
        real = tmp_path / "real.bin"
        real.write_bytes(b"x" * 8192)
        link = tmp_path / "link.bin"
        link.symlink_to(real)

        files = bench.collect_rootfs_files([str(tmp_path)], min_size=4096)
        paths = [f[0] for f in files]
        assert str(real) in paths
        assert str(link) not in paths


class TestStatIsRegular:
    def test_regular_file(self, tmp_path):
        f = tmp_path / "file.txt"
        f.write_text("hello")
        st = os.lstat(str(f))
        assert bench.stat_is_regular(st) is True

    def test_symlink(self, tmp_path):
        f = tmp_path / "file.txt"
        f.write_text("hello")
        link = tmp_path / "link.txt"
        link.symlink_to(f)
        st = os.lstat(str(link))
        assert bench.stat_is_regular(st) is False

    def test_directory(self, tmp_path):
        st = os.lstat(str(tmp_path))
        assert bench.stat_is_regular(st) is False


# ---------------------------------------------------------------------------
# Rootfs random read benchmark
# ---------------------------------------------------------------------------

class TestBenchRootfsRandRead:
    def test_no_files(self):
        result = bench.bench_rootfs_rand_read([], 100)
        assert result["count"] == 0
        assert "error" in result

    def test_reads_files(self, tmp_path):
        # Create a few files large enough for 4K reads
        for i in range(3):
            f = tmp_path / f"file_{i}.bin"
            f.write_bytes(os.urandom(8192))

        files = [(str(tmp_path / f"file_{i}.bin"), 8192) for i in range(3)]
        result = bench.bench_rootfs_rand_read(files, 50)
        assert "error" not in result
        assert result["count"] == 50
        assert result["files_sampled"] <= 3
        assert result["iops"] > 0
        assert result["throughput_mbps"] >= 0


# ---------------------------------------------------------------------------
# Rootfs sequential read benchmark
# ---------------------------------------------------------------------------

class TestBenchRootfsSeqRead:
    def test_reads_file(self, tmp_path):
        f = tmp_path / "test.bin"
        data = os.urandom(32768)
        f.write_bytes(data)

        result = bench.bench_rootfs_seq_read(str(f), 32768)
        assert result["file"] == str(f)
        assert result["size_bytes"] == 32768
        assert result["throughput_mbps"] >= 0
        assert result["duration_ms"] >= 0


# ---------------------------------------------------------------------------
# Startup benchmark -- time_command
# ---------------------------------------------------------------------------

class TestTimeCommand:
    def test_existing_command(self):
        t = bench.time_command(["python3", "--version"])
        assert t is not None
        assert t > 0

    def test_nonexistent_command(self):
        t = bench.time_command(["nonexistent_binary_xyz_123"])
        assert t is None


# ---------------------------------------------------------------------------
# Disk benchmark I/O tests
# ---------------------------------------------------------------------------

class TestDiskBench:
    def test_seq_write(self, tmp_path):
        testfile = str(tmp_path / "test.bin")
        result = bench.bench_seq_write(testfile, 1024 * 1024)
        assert result["size_bytes"] == 1024 * 1024
        assert result["throughput_mbps"] > 0
        assert os.path.exists(testfile)

    def test_seq_read(self, tmp_path):
        testfile = str(tmp_path / "test.bin")
        result = bench.bench_seq_read(testfile, 1024 * 1024)
        assert result["size_bytes"] == 1024 * 1024
        assert result["throughput_mbps"] > 0


# ---------------------------------------------------------------------------
# Main entrypoint tests
# ---------------------------------------------------------------------------

class TestMain:
    def test_unknown_mode_exits(self):
        old_argv = sys.argv
        try:
            sys.argv = ["capsem-bench", "bogus"]
            with pytest.raises(SystemExit) as exc_info:
                bench.main()
            assert exc_info.value.code == 1
        finally:
            sys.argv = old_argv

    def test_help_exits(self):
        old_argv = sys.argv
        try:
            sys.argv = ["capsem-bench", "--help"]
            with pytest.raises(SystemExit) as exc_info:
                bench.main()
            assert exc_info.value.code == 0
        finally:
            sys.argv = old_argv
