"""Measure, record, and say whether the machine was fit to measure on.

Capsem could not answer "how fast is it", "did my change make it worse", or
"did this release regress". Performance code lived in five places writing ten
incompatible JSON shapes, regression detection covered eight metrics across two
of eleven categories, and there was no entry point at all -- no `just bench`,
no bench binary on the host. 0.6.0 qualification failed on a gateway CPU
figure that no run had ever recorded, so the number could not be argued with.
A rerun showed it was a one-off. That ambiguity is what this removes.

The plan is deliberately short, because the interesting decisions are not
here: the binary owns the schema, the statistics and the ratchet, and a
collector owns nothing but raw samples. What this module owns is the two
things a plan is for -- that the binary exists before it is invoked, and that
measuring holds the machine, since a benchmark sharing a CPU with the rest of
a gate measures the sharing.
"""

from __future__ import annotations

from . import runtimeprepare
from .actions import Run
from .command import GateCommand
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step
from .plan import Plan


def _build(config: GateConfig, *, label: str = "bench.build") -> Step:
    """Build the harness before running it.

    Its absence is otherwise reported as a missing file at the moment of
    measurement, which reads like a benchmark failure and is not one.
    """
    settings = config.benchmark.run
    return step(
        label,
        Run(["cargo", "build", "-p", settings.crate, "--bin", settings.bin_name]),
        contends=(config.exclusive("workspace_binaries"),),
        produces=(config.path(settings.binary),),
        kind=Kind.PACKAGE,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )


def fitness(config: GateConfig) -> tuple[Step, Step]:
    """Describe the harness build and early machine-fitness check.

    The complete gate used to discover resident Capsem services only when its
    final timing suite ran, after spending more than an hour building assets.
    The Rust harness already owns machine fitness; preparation only needs to
    put that answer before expensive work.
    """
    settings = config.benchmark.run
    return (
        _build(config, label="benchmark-harness"),
        step(
            "benchmark-fitness",
            Run([str(config.path(settings.binary)), "doctor"]),
            contends=(config.exclusive("host_service"),),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.DISK}),
            speed=Speed.FAST,
        ),
    )


def _measure(config: GateConfig, *, quick: bool, dimensions: tuple[str, ...], commit: str):
    settings = config.benchmark.run
    timeout = settings.quick_timeout_secs if quick else settings.timeout_secs
    argv = [
        str(config.path(settings.binary)),
        "run",
        "--collectors",
        str(config.path(settings.collectors)),
        "--out",
        str(config.path(settings.store)),
        "--interpreter",
        settings.interpreter,
        "--timeout-secs",
        str(timeout),
        "--commit",
        commit,
    ]
    if quick:
        argv.append("--quick")
    argv.extend(dimensions)
    return step(
        "bench.quick" if quick else "bench.run",
        Run(argv),
        # Every dimension in one step and one contention: two benchmarks
        # sharing this machine would each measure the other.
        contends=(config.exclusive("host_service"),),
        kind=Kind.CAPSEM,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )


class BenchCommand(GateCommand, name="bench", help="measure performance and record it"):
    #: It builds the harness and writes the store, so it takes the machine
    #: lock like anything else that writes. Doubly so here: a benchmark
    #: sharing a machine with a second gate measures the sharing.
    exclusive = True

    @classmethod
    def add_arguments(cls, parser) -> None:
        # One argument, not `nargs="*"`: the recipe interpolates through
        # `{{quote(...)}}`, because double quotes still expand `$(...)` and
        # backticks. `just bench "x; echo pwned"` was a shell injection.
        parser.add_argument(
            "dimensions",
            nargs="?",
            default="",
            help="space-separated dimensions; every one with a collector when omitted",
        )
        parser.add_argument(
            "--quick",
            action="store_true",
            help="dev loop: only the dimensions that need no guest",
        )
        parser.add_argument("--commit", default="unknown")

    def plan(self) -> Plan:
        plan = Plan(self.name)
        prepared = runtimeprepare.prepare(
            plan,
            self._config,
            after=(),
            guest=not bool(self._args.quick),
            build_label="bench.build",
            sign_label="bench.sign",
        )
        plan.add(
            _measure(
                self._config,
                quick=bool(self._args.quick),
                dimensions=tuple(self._args.dimensions.split()),
                commit=self._args.commit,
            ),
            after=(prepared.ready,),
        )
        return plan


class BenchReportCommand(
    GateCommand,
    name="bench-report",
    help="what every measured subject reads, and how it has moved",
):
    #: Reading the store, but it builds the harness to do it.
    exclusive = True

    """Reading the store, which is the whole reason it is a store.

    The report it replaces was hand-written SVG bars in the docs citing two
    versions in a retired format, six weeks stale. This one is generated from
    the rows, so it is stale only if nothing has been measured.
    """

    def plan(self) -> Plan:
        settings = self._config.benchmark.run
        plan = Plan(self.name)
        built = plan.add(_build(self._config))
        plan.add(
            step(
                "bench.report",
                Run([
                    str(self._config.path(settings.binary)),
                    "report",
                    "--store",
                    str(self._config.path(settings.store)),
                ]),
                kind=Kind.CAPSEM,
                needs=frozenset({Needs.DISK}),
                speed=Speed.FAST,
            ),
            after=(built,),
        )
        return plan
