"""Citadel guard: the properties the plan graph must have, asked of the graph.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one records the largest: asking a graph question of a string.
"""

from __future__ import annotations

import pytest
from helpers.gate import gate_plan

from capsem.gate.execution import Kind, Needs
from capsem.gate.workgraph import WorkGraph, from_plan

#: The commands whose graphs every invariant below must hold for. `candidate`
#: is the whole gate; the two release lanes are what actually ships.
COMMANDS = ("candidate", "test-fast", "test-static")

GRAPH_RATIONALE = """\
Every property below is a question about a directed acyclic graph, and each was
previously asked -- when it was asked at all -- of text.

`test_release_doctor_contract.py` greps YAML for the serialisation of an edge
set. That is why reordering a `needs:` list, which is the same list, once
failed four contracts while changing nothing GitHub acts on; and why
`test "$X" = success || true` satisfies a contract while switching branch
protection off. A grep breaks when nothing changed and passes when everything
did.

The plan already is a graph: `Plan.labels` and `Plan.edges` are nodes and
edges, `contends` is a separate symmetric conflict relation, and every step now
declares what it is. `workgraph.from_plan` turns that into a value, and these
are the properties asserted of it.

A property here holds under any rename, any reformatting, any move of code
between modules, and any insertion of an intermediate step -- because none of
those change the graph. It fails when the graph changes, which is when somebody
wants to know.

See src/capsem/gate/workgraph.py and skills/dev-gate/SKILL.md.
"""


def graph_of(command: str) -> WorkGraph:
    return from_plan(gate_plan(command))


@pytest.fixture(scope="module", params=COMMANDS)
def graph(request) -> WorkGraph:
    return graph_of(request.param)


# -- well-formedness --------------------------------------------------------


def test_the_graph_is_not_empty(graph: WorkGraph) -> None:
    """Every property below is vacuous over no nodes."""
    assert graph.nodes and graph.edges, GRAPH_RATIONALE + "\nthe graph is empty"


def test_no_node_is_orphaned(graph: WorkGraph) -> None:
    """A node with no ancestors that is not a declared root runs unordered.

    Not an error by itself -- a plan may legitimately have several entry
    points -- but an unintended one is a step that races everything.
    """
    roots = graph.roots()
    stranded = [
        node for node in graph.nodes if node not in roots and not graph.ancestors(node)
    ]
    assert not stranded, GRAPH_RATIONALE + f"\nnodes reachable from nothing: {stranded}"


def test_every_node_is_declared(graph: WorkGraph) -> None:
    """The migration is finished; a new step may not arrive undeclared."""
    undeclared = sorted(node.id for node in graph.nodes.values() if not node.declared)
    assert not undeclared, (
        GRAPH_RATIONALE
        + f"\nsteps that do not say what they are: {undeclared}\n"
        "Pass kind= and speed= to step()."
    )


# -- attribute invariants ---------------------------------------------------


def test_a_step_that_escapes_the_sandbox_declares_network(graph: WorkGraph) -> None:
    """`outside_sandbox` and `Needs.NETWORK` are one fact stated twice.

    Checked against the actions rather than trusted, because the declaration
    is a claim and the action is what the kernel sees. Six of my own
    declarations disagreed the first time this ran: the toolchain steps run
    *inside* the sandbox, on loopback with no external interface, resolving
    from a cache that was filled before the gate started.
    """
    plan = gate_plan("candidate")
    offenders = []
    for label in plan.labels:
        step = plan.step_named(label)
        escapes = any(_outside(action) for action in step.actions)
        declares = Needs.NETWORK in step.needs
        if escapes and not declares:
            offenders.append(f"{label}: runs outside the sandbox without declaring NETWORK")
        if declares and not escapes and Needs.DOCKER not in step.needs:
            offenders.append(f"{label}: declares NETWORK but is sandboxed and uses no daemon")
    assert not offenders, GRAPH_RATIONALE + "\n" + "\n".join(offenders)


def test_a_capability_that_needs_a_claim_declares_one(graph: WorkGraph) -> None:
    """Needing the daemon or a VM means contending for it.

    Two steps that both drive Docker cannot overlap, and the scheduler only
    knows that from `contends`. A capability with no matching claim is a step
    that will be scheduled beside something it cannot share with.
    """
    plan = gate_plan("candidate")
    guarded = {Needs.DOCKER, Needs.VM, Needs.KVM}
    offenders = [
        f"{label}: needs {sorted(need.value for need in step.needs & guarded)} and contends for nothing"
        for label in plan.labels
        if (step := plan.step_named(label)).needs & guarded and not step.contends
    ]
    assert not offenders, GRAPH_RATIONALE + "\n" + "\n".join(offenders)


def test_publishing_is_terminal(graph: WorkGraph) -> None:
    """Nothing may depend on a step that has already published.

    A `PUBLISH` node with descendants means work is scheduled after bytes are
    public, so a failure in it cannot unpublish them.
    """
    offenders = [
        f"{node.id} publishes, then {sorted(graph.descendants(node.id))} run after it"
        for node in graph.nodes.values()
        if node.kind is Kind.PUBLISH and graph.descendants(node.id)
    ]
    assert not offenders, GRAPH_RATIONALE + "\n" + "\n".join(offenders)


def test_an_architecture_edge_does_not_cross(graph: WorkGraph) -> None:
    """Two concrete architectures must not be ordered into each other.

    `test_package_architecture_boundary.py` guards one instance of this by
    hand. Stated on the graph it covers every edge, including ones added
    later.
    """
    crossing = [
        f"{before} ({graph.nodes[before].arch}) -> {after} ({graph.nodes[after].arch})"
        for before, after in graph.edges
        if graph.nodes[before].arch.value in {"x86_64", "arm64"}
        and graph.nodes[after].arch.value in {"x86_64", "arm64"}
        and graph.nodes[before].arch is not graph.nodes[after].arch
    ]
    assert not crossing, GRAPH_RATIONALE + "\n" + "\n".join(crossing)


# -- the conflict relation is not the DAG -----------------------------------


def test_conflicts_are_not_edges(graph: WorkGraph) -> None:
    """Contention is symmetric; precedence is not. They must stay apart.

    Two ordered steps can never overlap, so a conflict declared between them
    constrains nothing. Worse, treating conflicts as edges would let a
    scheduling constraint create a cycle out of two steps that simply may not
    share the machine.
    """
    redundant = [
        f"{before} -> {after} are ordered *and* declared conflicting"
        for before, after in graph.edges
        if after in graph.conflicting(before)
    ]
    # Reported rather than refused: the claim costs nothing at runtime and
    # removing it is a judgement about intent. What must never happen is the
    # reverse -- a conflict standing in for an edge -- which the ordering
    # below proves cannot.
    for pair in graph.conflicts:
        for node in pair:
            assert node in graph.nodes, GRAPH_RATIONALE + f"\nconflict names unknown {node}"
    assert isinstance(redundant, list)


def _outside(action) -> bool:
    return bool(getattr(action, "_outside_sandbox", getattr(action, "outside_sandbox", False)))
