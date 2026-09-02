#!/usr/bin/env python3
"""Enforce the intended dependency edges between workspace crates."""

from __future__ import annotations

import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent

# Keep this list explicit: adding a workspace crate or an internal dependency is
# an architecture decision and must update this contract and docs/architecture.md.
ALLOWED_DEPENDENCIES = {
    "toyoterm-api": set(),
    "toyoterm-app": {
        "toyoterm-api",
        "toyoterm-config",
        "toyoterm-ipc",
        "toyoterm-mux",
        "toyoterm-pty",
        "toyoterm-render",
        "toyoterm-script",
        "toyoterm-terminal",
    },
    "toyoterm-cli": {
        "toyoterm-api",
        "toyoterm-app",
        "toyoterm-ipc",
        "toyoterm-mux",
        "toyoterm-pty",
        "toyoterm-terminal",
    },
    "toyoterm-config": set(),
    "toyoterm-ipc": {"toyoterm-api"},
    "toyoterm-mux": {"toyoterm-api"},
    "toyoterm-pty": set(),
    "toyoterm-render": {"toyoterm-api", "toyoterm-mux", "toyoterm-terminal"},
    "toyoterm-script": {"toyoterm-api", "toyoterm-config"},
    "toyoterm-terminal": set(),
}

ALLOWED_DEV_DEPENDENCIES = {
    ("toyoterm-pty", "toyoterm-terminal"),
    ("toyoterm-script", "toyoterm-mux"),
    ("toyoterm-script", "toyoterm-pty"),
}


def load_metadata() -> dict:
    command = [
        "cargo",
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--no-deps",
    ]
    result = subprocess.run(
        command,
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def find_cycle(graph: dict[str, set[str]]) -> list[str] | None:
    visiting: set[str] = set()
    visited: set[str] = set()
    path: list[str] = []

    def visit(crate: str) -> list[str] | None:
        if crate in visiting:
            start = path.index(crate)
            return path[start:] + [crate]
        if crate in visited:
            return None

        visiting.add(crate)
        path.append(crate)
        for dependency in sorted(graph[crate]):
            cycle = visit(dependency)
            if cycle is not None:
                return cycle
        path.pop()
        visiting.remove(crate)
        visited.add(crate)
        return None

    for crate in sorted(graph):
        cycle = visit(crate)
        if cycle is not None:
            return cycle
    return None


def main() -> int:
    metadata = load_metadata()
    workspace_ids = set(metadata["workspace_members"])
    packages = {
        package["name"]: package
        for package in metadata["packages"]
        if package["id"] in workspace_ids
    }
    workspace_crates = set(packages)
    errors: list[str] = []

    missing_contracts = workspace_crates - set(ALLOWED_DEPENDENCIES)
    stale_contracts = set(ALLOWED_DEPENDENCIES) - workspace_crates
    for crate in sorted(missing_contracts):
        errors.append(f"workspace crate {crate} has no architecture contract")
    for crate in sorted(stale_contracts):
        errors.append(f"architecture contract references missing crate {crate}")

    production_graph: dict[str, set[str]] = defaultdict(set)
    actual_dev_dependencies: set[tuple[str, str]] = set()
    for crate, package in packages.items():
        production_graph[crate]
        for dependency in package["dependencies"]:
            dependency_name = dependency["name"]
            if dependency_name not in workspace_crates:
                continue

            kind = dependency["kind"] or "normal"
            edge = (crate, dependency_name)
            if kind == "normal":
                production_graph[crate].add(dependency_name)
                if dependency_name not in ALLOWED_DEPENDENCIES.get(crate, set()):
                    errors.append(
                        f"forbidden production dependency: {crate} -> {dependency_name}"
                    )
            elif kind == "dev":
                actual_dev_dependencies.add(edge)
                if edge not in ALLOWED_DEV_DEPENDENCIES:
                    errors.append(
                        f"unreviewed dev dependency: {crate} -> {dependency_name}"
                    )
            else:
                errors.append(
                    f"internal {kind} dependency is not allowed: {crate} -> {dependency_name}"
                )

    stale_dev_edges = ALLOWED_DEV_DEPENDENCIES - actual_dev_dependencies
    for crate, dependency in sorted(stale_dev_edges):
        errors.append(f"stale dev dependency contract: {crate} -> {dependency}")

    cycle = find_cycle(production_graph)
    if cycle is not None:
        errors.append(f"production dependency cycle: {' -> '.join(cycle)}")

    if errors:
        print("crate architecture check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        print(
            "Update the manifests and the documented contract together.",
            file=sys.stderr,
        )
        return 1

    edge_count = sum(len(dependencies) for dependencies in production_graph.values())
    print(
        f"crate architecture check: {len(workspace_crates)} crates, "
        f"{edge_count} production edges, no cycles"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
