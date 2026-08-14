"""The graph of gate work, separated from whatever spelling produced it.

Named `workgraph` rather than `releasegraph`, which is taken: that module
authors a release *manifest* graph for the install proof. This one is the DAG
of work -- gate steps, CI jobs, documented stages.

Every question worth asking about the release is a question about that DAG: may
this run in the fast lane, is this reachable without the network, do these two
ever overlap, does the documentation describe the order the gate actually uses.
A `Plan`, a recorded `PlanShape` and a workflow's `needs:` are three spellings
of one structure, so each gets a function into this model and the questions are
asked here.

Asked of the artifacts instead they become string comparisons. Grepping
`needs: [a, b, c]` asserts on the *serialisation* of an unordered edge set,
which is why reordering that list -- the same list -- once failed four
contracts while changing nothing GitHub acts on.

Two relations, deliberately apart. Edges are precedence and form the DAG.
Contention is symmetric and non-transitive: two steps that may not overlap have
no order between them, so folding it into the edge set would be a category
error and could manufacture a cycle out of a scheduling constraint.
"""

from __future__ import annotations

from enum import StrEnum

from .configschema import Strict
from .execution import SATURATES, Arch, Kind, Needs, Requires, Speed


class Origin(StrEnum):
    """Which artifact a node came from.

    Kept on the node so two graphs are never merged by accident. A gate step
    and a CI job can share a name and mean different things; a comparison
    between them has to be a stated correspondence, not a coincidence of
    labels.
    """

    GATE = "gate"
    WORKFLOW = "workflow"
    DOCS = "docs"


class Node(Strict):
    """One unit of work, described by what it is rather than what it is called."""

    id: str
    origin: Origin
    stage: str
    kind: Kind
    needs: frozenset[Needs]
    arch: Arch
    speed: Speed
    concurrency: int

    @property
    def declared(self) -> bool:
        return self.kind is not Kind.UNDECLARED and self.speed is not Speed.UNDECLARED

    def workers(self, cores: int) -> int:
        """Machine this node takes, resolving `SATURATES` against a real host.

        Resolved here rather than at declaration because plan construction is
        inert and may not read the machine, and because one graph is analysed
        against runs from hosts with different core counts.
        """
        return cores if self.concurrency == SATURATES else self.concurrency


class WorkGraph(Strict):
    """A typed DAG, plus the conflict relation that is not part of it."""

    nodes: dict[str, Node]
    edges: dict[tuple[str, str], Requires]
    """`(before, after)` -> why it exists. Precedence only."""

    conflicts: frozenset[frozenset[str]]
    """Sets of nodes that may not overlap. Symmetric, non-transitive."""

    # -- the relations, computed rather than stored ------------------------

    def predecessors(self) -> dict[str, set[str]]:
        """`after -> {before}`, the shape `graphlib` and `longest_chain` want."""
        found: dict[str, set[str]] = {node: set() for node in self.nodes}
        for before, after in self.edges:
            found[after].add(before)
        return found

    def successors(self) -> dict[str, set[str]]:
        found: dict[str, set[str]] = {node: set() for node in self.nodes}
        for before, after in self.edges:
            found[before].add(after)
        return found

    def ancestors(self, node: str) -> frozenset[str]:
        """Everything that must finish before `node` may start, transitively."""
        return _closure(node, self.predecessors())

    def descendants(self, node: str) -> frozenset[str]:
        return _closure(node, self.successors())

    def reaches(self, before: str, after: str) -> bool:
        """Whether `after` depends on `before` by any path.

        Reachability, not adjacency: a contract requiring one step to precede
        another must keep holding when somebody inserts a step between them.
        """
        return before in self.ancestors(after)

    # -- derived properties ------------------------------------------------

    def hermetic(self, node: str) -> bool:
        """Whether this node's work is closed under the bytes it consumes.

        Derived, never declared: a flag would let a node claim a property its
        inputs contradict, which is the claim no reader can check at one call
        site.

        Contamination follows `ARTIFACT` edges only. A node that *consumes*
        bytes an upstream step downloaded is not hermetic; a node that merely
        runs *after* one is unaffected, because sequence is not provenance.
        The first version walked every edge and reported 86 of 110 nodes as
        non-hermetic -- the whole graph downstream of three advisory queries
        that sit early in the fast phase and hand nothing to anybody. That
        answer was useless, and wrong in the direction that makes a real
        contamination impossible to see.

        `UNDECLARED` edges are treated as `ARTIFACT`, the conservative
        reading: while the edge-kind migration is unfinished this can call a
        hermetic node dirty, never a dirty one hermetic.
        """
        carrying = {Requires.ARTIFACT, Requires.UNDECLARED}
        upstream: dict[str, set[str]] = {name: set() for name in self.nodes}
        for (before, after), why in self.edges.items():
            if why in carrying:
                upstream[after].add(before)
        return all(
            Needs.NETWORK not in self.nodes[reached].needs
            for reached in (node, *_closure(node, upstream))
        )

    def roots(self) -> frozenset[str]:
        return frozenset(node for node, before in self.predecessors().items() if not before)

    def leaves(self) -> frozenset[str]:
        return frozenset(node for node, after in self.successors().items() if not after)

    def redundant_edges(self) -> dict[tuple[str, str], Requires]:
        """Edges implied by a longer path -- not in the transitive reduction.

        `u -> v` is redundant when `v` is still reachable from `u` without it.
        The graph schedules identically without it, so its only effect is to
        state a constraint twice; and if the two ever disagree, nothing says
        which was meant.
        """
        successors = self.successors()
        redundant: dict[tuple[str, str], Requires] = {}
        for (before, after), why in self.edges.items():
            reduced = {node: set(onward) for node, onward in successors.items()}
            reduced[before].discard(after)
            if after in _closure(before, reduced):
                redundant[(before, after)] = why
        return redundant

    def conflicting(self, node: str) -> frozenset[str]:
        """Nodes this one may not overlap with."""
        return frozenset(
            other
            for group in self.conflicts
            if node in group
            for other in group
            if other != node
        )


def _closure(start: str, relation: dict[str, set[str]]) -> frozenset[str]:
    """Everything reachable from `start`, excluding itself unless cyclic."""
    seen: set[str] = set()
    pending = list(relation.get(start, ()))
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        pending.extend(relation.get(current, ()))
    return frozenset(seen)


def from_plan(plan) -> WorkGraph:
    """A gate plan as a work graph.

    Reads what the plan already exposes -- `labels`, `edges`, the `Step` on
    each node, and the stage the phase recorded -- so this is a translation
    rather than a second opinion. `test_workgraph.py` asserts the edge set
    round-trips exactly, because a functor that quietly drops an edge would
    make every property computed here true of a graph nobody runs.

    Edges arrive `UNDECLARED`: `Plan.add(after=...)` takes steps, not kinds,
    and inventing a kind from the predecessor's `produces` would be inference
    of the sort this model exists to remove.
    """
    nodes = {
        label: Node(
            id=label,
            origin=Origin.GATE,
            stage=plan.stage_of(label),
            kind=plan.step_named(label).kind,
            needs=plan.step_named(label).needs,
            arch=plan.step_named(label).arch,
            speed=plan.step_named(label).speed,
            concurrency=plan.step_named(label).concurrency,
        )
        for label in plan.labels
    }
    # `contends` names an exclusive; two steps conflict when they name the same
    # one and at least one holds it unshared. Grouped by exclusive rather than
    # enumerated pairwise, so the relation stays what it is -- a set of things
    # that cannot overlap -- instead of becoming an edge list by accident.
    holders: dict[str, set[str]] = {}
    for label in plan.labels:
        for exclusive in plan.step_named(label).contends:
            holders.setdefault(exclusive.name, set()).add(label)
    return WorkGraph(
        nodes=nodes,
        edges={edge: plan.requires_of(*edge) for edge in plan.edges},
        conflicts=frozenset(frozenset(group) for group in holders.values() if len(group) > 1),
    )
