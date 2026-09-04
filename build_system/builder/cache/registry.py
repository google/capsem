"""Resolve cache IDs to typed backends without leaking storage mechanics."""

from __future__ import annotations

from .api import CacheBackend, CacheMutationResult, CacheOperation, CacheRequest
from .contract import CacheContract
from .dockerimages import image_cache_size, plan_image_cache, plan_image_clean
from .enforcement import enforce_repository, enforce_runtime
from .inventory import scan_inventory, select_inventory
from .models import CachePolicy
from .operations import apply_prune
from .paths import CachePaths
from .planner import plan_clean, plan_prune
from .runtimeexec import CommandRunner, execute
from .runtimeinventory import scan_runtimes
from .runtimeoperations import apply_runtime_prune
from .runtimeplanner import plan_runtime_clean, plan_runtime_prune
from .stats import CacheStats, CacheUsage, build_stats


class DiskBackend:
    """Repository-directory implementation of the common cache API."""

    def __init__(self, paths: CachePaths, policy: CachePolicy) -> None:
        self._paths = paths
        self._policy = policy

    @property
    def cache_ids(self) -> frozenset[str]:
        return frozenset(self._policy.stages)

    def contract(self, cache_id: str) -> CacheContract:
        try:
            source = self._policy.stages[cache_id]
        except KeyError:
            raise ValueError(f"unknown disk cache {cache_id!r}") from None
        return CacheContract.from_owner(source)

    def usages(self) -> tuple[CacheUsage, ...]:
        report = build_stats(scan_inventory(self._paths, self._policy), self._policy)
        return tuple(item for item in report.caches if item.cache_id in self.cache_ids)

    def mutate(self, request: CacheRequest) -> CacheMutationResult:
        if request.cache_id != "all" and request.cache_id not in self.cache_ids:
            raise ValueError(f"unknown disk cache {request.cache_id!r}")
        if request.operation is CacheOperation.ENFORCE:
            result = enforce_repository(
                self._paths, self._policy, request.cache_id, reason=request.reason
            )
            return CacheMutationResult(
                cache_id=request.cache_id,
                operation=request.operation,
                before_size_bytes=result.before_size_bytes,
                after_size_bytes=result.after_size_bytes,
                reclaim_bytes=result.reclaim_bytes,
                action_count=result.action_count,
                applied=True,
                violations=result.violations,
            )
        inventory = select_inventory(scan_inventory(self._paths, self._policy), request.cache_id)
        before = inventory.logical_bytes
        plan = (
            plan_clean(inventory, request.cache_id)
            if request.operation is CacheOperation.CLEAN
            else plan_prune(inventory, self._policy)
        )
        if request.apply and plan.actions:
            apply_prune(self._paths, plan, reason=request.reason)
        after = (
            select_inventory(
                scan_inventory(self._paths, self._policy), request.cache_id
            ).logical_bytes
            if request.apply
            else before
        )
        return CacheMutationResult(
            cache_id=request.cache_id,
            operation=request.operation,
            before_size_bytes=before,
            after_size_bytes=after,
            reclaim_bytes=plan.reclaim_bytes,
            action_count=len(plan.actions),
            applied=request.apply,
            violations=plan.violations,
        )


class RuntimeBackend:
    """Docker/Colima or Tart implementation of the common cache API."""

    def __init__(
        self,
        paths: CachePaths,
        policy: CachePolicy,
        runtime_id: str,
        *,
        runner: CommandRunner = execute,
    ) -> None:
        self._paths = paths
        self._policy = policy
        self._runtime_id = runtime_id
        self._runner = runner

    @property
    def runtime_id(self) -> str:
        return self._runtime_id

    @property
    def cache_ids(self) -> frozenset[str]:
        values = {self._runtime_id}
        if (
            self._policy.control is not None
            and self._runtime_id == self._policy.control.docker.runtime_id
        ):
            values.update(self._policy.control.docker.images)
        return frozenset(values)

    def contract(self, cache_id: str) -> CacheContract:
        if cache_id == self._runtime_id:
            source = self._policy.runtimes[cache_id]
        elif (
            self._policy.control is not None
            and cache_id in self._policy.control.docker.images
            and self._runtime_id == self._policy.control.docker.runtime_id
        ):
            source = self._policy.control.docker.images[cache_id]
        else:
            raise ValueError(f"runtime backend {self._runtime_id!r} cannot own {cache_id!r}")
        return CacheContract.from_owner(source)

    def _snapshot(self):
        return scan_runtimes(
            self._policy,
            runner=self._runner,
            runtime_ids=frozenset({self._runtime_id}),
        )

    def usages(self) -> tuple[CacheUsage, ...]:
        inventory = scan_inventory(self._paths, self._policy)
        inventory = inventory.model_copy(update={"runtimes": self._snapshot().runtimes})
        report = build_stats(inventory, self._policy)
        return tuple(item for item in report.caches if item.cache_id in self.cache_ids)

    def mutate(self, request: CacheRequest) -> CacheMutationResult:
        if request.cache_id not in self.cache_ids:
            raise ValueError(
                f"runtime backend {self._runtime_id!r} cannot own {request.cache_id!r}"
            )
        child = request.cache_id != self._runtime_id
        if request.operation is CacheOperation.ENFORCE and not child:
            result = enforce_runtime(
                self._paths,
                self._policy,
                self._runtime_id,
                reason=request.reason,
                runner=self._runner,
            )
            return CacheMutationResult(
                cache_id=request.cache_id,
                operation=request.operation,
                before_size_bytes=result.before_size_bytes,
                after_size_bytes=result.after_size_bytes,
                reclaim_bytes=result.reclaim_bytes,
                action_count=result.action_count,
                applied=True,
                violations=result.violations,
            )
        before_snapshot = self._snapshot()
        inventory = before_snapshot.runtimes[0]
        image = (
            self._policy.control.docker.images[request.cache_id]
            if child and self._policy.control is not None
            else None
        )
        before = image_cache_size(inventory, image) if image is not None else inventory.owned_bytes
        plan = (
            plan_image_clean(before_snapshot, self._policy, request.cache_id)
            if child and request.operation is CacheOperation.CLEAN
            else plan_image_cache(before_snapshot, self._policy, request.cache_id)
            if child
            else plan_runtime_clean(before_snapshot, self._policy)
            if request.operation is CacheOperation.CLEAN
            else plan_runtime_prune(before_snapshot, self._policy)
        )
        failures = []
        if request.apply and plan.actions:
            result = apply_runtime_prune(
                self._paths,
                self._policy,
                plan,
                reason=request.reason,
                runner=self._runner,
            )
            failures.extend(item.output for item in result.results if item.returncode != 0)
        after_inventory = self._snapshot().runtimes[0] if request.apply else inventory
        after = (
            image_cache_size(after_inventory, image)
            if image is not None
            else after_inventory.owned_bytes
        )
        return CacheMutationResult(
            cache_id=request.cache_id,
            operation=request.operation,
            before_size_bytes=before,
            after_size_bytes=after,
            reclaim_bytes=plan.reclaim_bytes,
            action_count=len(plan.actions),
            applied=request.apply,
            violations=(*plan.violations, *failures),
        )


class CacheRegistry:
    """One typed entry point for every configured cache owner."""

    def __init__(
        self, paths: CachePaths, policy: CachePolicy, *, runner: CommandRunner = execute
    ) -> None:
        self._paths = paths
        self._policy = policy
        self._disk = DiskBackend(paths, policy)
        self._runtimes = tuple(
            RuntimeBackend(paths, policy, runtime_id, runner=runner)
            for runtime_id in sorted(policy.runtimes)
        )
        self._backends: tuple[CacheBackend, ...] = (self._disk, *self._runtimes)

    @property
    def cache_ids(self) -> frozenset[str]:
        return frozenset(cache_id for backend in self._backends for cache_id in backend.cache_ids)

    def _backend(self, cache_id: str) -> CacheBackend:
        matches = [backend for backend in self._backends if cache_id in backend.cache_ids]
        if len(matches) != 1:
            known = ", ".join(sorted(self.cache_ids))
            raise ValueError(f"unknown cache {cache_id!r}; expected one of: {known}")
        return matches[0]

    def contract(self, cache_id: str) -> CacheContract:
        """Return one owner's common contract without exposing its backend."""
        return self._backend(cache_id).contract(cache_id)

    def stats(self, *, offline: bool = False) -> CacheStats:
        inventory = scan_inventory(self._paths, self._policy)
        snapshot = scan_runtimes(self._policy, offline=offline)
        return build_stats(
            inventory.model_copy(update={"runtimes": snapshot.runtimes}),
            self._policy,
            unavailable_is_violation=not offline,
        )

    def mutate(self, request: CacheRequest) -> tuple[CacheMutationResult, ...]:
        if request.cache_id != "all":
            return (self._backend(request.cache_id).mutate(request),)
        results = [self._disk.mutate(request.model_copy(update={"cache_id": "all"}))]
        results.extend(
            backend.mutate(request.model_copy(update={"cache_id": backend.runtime_id}))
            for backend in self._runtimes
        )
        return tuple(results)
