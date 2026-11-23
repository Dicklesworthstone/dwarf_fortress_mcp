#!/usr/bin/env python3
"""Run the real resource-project owner and Manager child against native TCP doubles.

The joined endpoint routes native plugin frames to the existing Manager and
Workshop peers. Rust alone creates parent and child custody. Source rebasing is
explicit fixture construction, not native bridge, modern stdio or live evidence.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import socket
import struct
import subprocess
import sys
import tempfile
import threading

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
PHASE = "RESOURCE_PROJECT_MANAGER_PHASE"
CASE = "RESOURCE_PROJECT_MANAGER_CASE"
INPUT = "RESOURCE_PROJECT_MANAGER_INPUT"
OUTPUT = "RESOURCE_PROJECT_MANAGER_OUTPUT"


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot(directory):
    return {path.name: (path.stat().st_mode & 0o7777, hashlib.sha256(path.read_bytes()).hexdigest())
            for path in directory.iterdir() if path.is_file()}


def custody(directory):
    return {str(path.relative_to(directory)): snapshot(path) for path in (
        directory / "project", directory / "manager/custody", directory / "workshop/custody",
        directory / "production/parent", directory / "production/workforce", directory / "production/orders")}


class JoinedEndpoint:
    """Route each native connection by its first checked plugin binding.

    Request and reply bodies are forwarded unchanged. The common DFHack greeting
    is repeated and checked against the selected existing native protocol double.
    Every connection and relay worker is joined on close.
    """
    def __init__(self, manager, workshop, codec):
        self.targets = {b"dfmcp_manager_v1_23": manager.endpoint,
                        b"dfmcp_workshop_v1_20": workshop.endpoint}
        self.codec = codec
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.listener.settimeout(.1)
        self.endpoint = f"127.0.0.1:{self.listener.getsockname()[1]}"
        self.stop = threading.Event()
        self.errors, self.routes, self.workers, self.sockets = [], [], [], []
        self.thread = threading.Thread(target=self.run, name="owned-resource-project-endpoint")
        self.thread.start()

    def relay(self, client):
        upstream = None
        try:
            client.settimeout(10)
            assert self.codec.receive(client, 12) == b"DFHack?\n\x01\0\0\0"
            client.sendall(b"DFHack!\n\x01\0\0\0")
            header = self.codec.receive(client, 8)
            method, padding, size = struct.unpack("<hhI", header)
            assert method == 0 and padding == 0 and size <= 2048
            body = self.codec.receive(client, size)
            request = self.codec.fields(body)
            assert request[1] == b"Handshake" and request[4] in self.targets
            self.routes.append(request[4].decode())
            host, port = self.targets[request[4]].rsplit(":", 1)
            upstream = socket.create_connection((host, int(port)), timeout=10)
            self.sockets.append(upstream)
            upstream.sendall(b"DFHack?\n\x01\0\0\0")
            assert self.codec.receive(upstream, 12) == b"DFHack!\n\x01\0\0\0"
            upstream.sendall(header + body)
            with selectors.DefaultSelector() as selector:
                selector.register(client, selectors.EVENT_READ, upstream)
                selector.register(upstream, selectors.EVENT_READ, client)
                while not self.stop.is_set():
                    for key, _ in selector.select(.1):
                        data = key.fileobj.recv(65536)
                        if not data:
                            return
                        key.data.sendall(data)
        except (OSError, EOFError) as cause:
            if not self.stop.is_set():
                self.errors.append(repr(cause))
        except BaseException as cause:
            self.errors.append(repr(cause))
        finally:
            client.close()
            if upstream is not None:
                upstream.close()

    def run(self):
        try:
            while not self.stop.is_set():
                try:
                    client, _ = self.listener.accept()
                except socket.timeout:
                    continue
                self.sockets.append(client)
                worker = threading.Thread(target=self.relay, args=(client,),
                                          name="owned-resource-project-relay")
                self.workers.append(worker)
                worker.start()
        finally:
            self.listener.close()

    def close(self):
        self.stop.set()
        for stream in self.sockets:
            try:
                stream.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(6)
        assert not self.thread.is_alive(), "joined endpoint did not drain"
        for worker in self.workers:
            worker.join(6)
            assert not worker.is_alive(), "native relay did not drain"
        assert not self.errors, self.errors


def child(executable, test, phase, case, environment, source, output):
    environment = environment.copy()
    environment.update({PHASE: phase, CASE: case, INPUT: str(source), OUTPUT: str(output),
                        "PYTHONDONTWRITEBYTECODE": "1"})
    result = subprocess.run([executable, "--exact", test, "--nocapture"], env=environment,
                            timeout=180, capture_output=True, text=True, check=False)
    assert result.returncode == 0, f"{phase}/{case}:\n{result.stdout}\n{result.stderr}"
    assert f"resource-project-manager:{phase}:{case}:checked" in result.stdout


def reopened(environment, retained):
    environment = environment.copy()
    reference = retained["reference"]
    parent = json.loads(environment["DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG"])
    parent["initialize"] = False
    parent["acknowledged"] = reference["parent_acknowledged"]
    environment["DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG"] = json.dumps(parent)
    checkpoint = reference["retained_stage"]["checkpoint"]
    for key, stage in (("DFMCP_MANAGER_V1_23_MCP_CONFIG", "manager"),
                       ("DFMCP_WORKSHOP_MCP_CONFIG", "workshop")):
        if checkpoint.get(stage) is not None:
            configured = json.loads(environment[key])
            configured["initialize"] = False
            configured["acknowledged"] = checkpoint[stage]["acknowledged"]
            if stage == "manager" and checkpoint[stage].get("production_handoff_floor") is not None:
                configured["production_handoff_floor"] = checkpoint[stage]["production_handoff_floor"]
            environment[key] = json.dumps(configured)
    return environment


def run_case(executable, test, case):
    manager = load("resource_project_manager_native", "manager_appointment.py")
    workshop = load("resource_project_workshop_native", "workshop.py")
    combined = load("resource_project_production_config", "combined_production.py")
    helper = load("resource_project_manager_vectors", "combined_manager_prerequisite_runtime_peer.py")
    manager.MINOR, manager.PROFILE = 23, "1.23"
    workshop.VECTORS = {key: bytes.fromhex(value) for key, value in
                        json.loads((ROOT / "tests/native/workshop/vectors.json").read_text()).items()}
    workshop_capture = workshop.VECTORS["capture"]
    site = int.from_bytes(workshop_capture[32:36], "big")
    dimensions = [int.from_bytes(workshop_capture[pos:pos + 4], "big") for pos in (36, 40, 44)]
    original = {key: bytes.fromhex(value) for key, value in
                json.loads((ROOT / "tests/fixtures/manager_v1_23_component.json").read_text()).items()}
    vectors = helper.rebase_manager(original, site, dimensions)
    after_at, after_size = helper.record_captures(vectors["assigned"])[1]
    ready = vectors["assigned"][after_at:after_at + after_size]
    tick = int.from_bytes(ready[24:32], "big")
    # Workshop is only opened/read in this campaign. No rebased build plan or
    # receipt is provided; the original peer rejects every accidental writer.
    if case != "stale_workshop":
        capture = bytearray(workshop_capture)
        capture[24:32] = tick.to_bytes(8, "big")
        workshop.VECTORS["capture"] = bytes(capture)
    combined.SITE, combined.DF, combined.DFH, combined.TICK = site, b"test-df", b"test-dfhack", tick

    with tempfile.TemporaryDirectory(prefix="dfmcp-resource-project-manager-") as temporary:
        directory = Path(temporary).resolve()
        for path in (directory / "project", directory / "manager", directory / "manager/custody",
                     directory / "workshop", directory / "workshop/custody", directory / "production"):
            path.mkdir(mode=0o700)

        class ManagerPeer(manager.Peer):
            damaged_ack = None
            donor_at_export = None

            def reply(self, stream, raw):
                fields = manager.fields(raw)
                assigned_reply = self.writer_count and 10 in fields
                export_read = self.writer_count and 9 in fields
                if export_read:
                    fields[9] = ready
                    raw = b"".join(manager.number(tag, value) if isinstance(value, int)
                                   else manager.blob(tag, value) for tag, value in fields.items())
                    if self.donor_at_export is None:
                        self.donor_at_export = snapshot(directory / "manager/custody")
                inject = (case == "parent_ack" and assigned_reply) or (case == "handoff_ack" and export_read)
                if inject and self.damaged_ack is None:
                    path = directory / "project/resource-project.ack"
                    self.damaged_ack = path.read_bytes()
                    assert self.damaged_ack
                    damaged = bytearray(self.damaged_ack)
                    damaged[-1] ^= 1
                    path.write_bytes(damaged)
                super().reply(stream, raw)

        manager_peer = ManagerPeer(directory / "manager", vectors)
        workshop_peer = workshop.Peer(directory / "workshop")
        endpoint = JoinedEndpoint(manager_peer, workshop_peer, manager)
        try:
            if case == "lost_commit":
                manager_peer.drop_commit = True
                manager_peer.release_commit.set()
            manager_config = manager.config(directory / "manager", manager_peer,
                endpoint=endpoint.endpoint, site_id=site, map_dimensions=dimensions,
                read_scope=[0, 0, 0] + [n - 1 for n in dimensions],
                scope=[0, 0, 0] + [n - 1 for n in dimensions])
            workshop_config = workshop.config(directory / "workshop", workshop_peer,
                endpoint=endpoint.endpoint, authority_expires_tick=tick + 12000)
            environment = combined.environment(directory / "production", endpoint.endpoint)
            for child_environment in (manager.environment(manager_config), workshop.environment(workshop_config)):
                environment.update({key: value for key, value in child_environment.items() if key.startswith("DFMCP_")})
            environment.update({
                "DFMCP_ALLOW_UNADMITTED_NATIVE_RESOURCE_PROJECT": "1",
                "DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG": json.dumps({
                    "schema": "dfmcp.native-resource-project-development/1",
                    "directory": str(directory / "project"), "initialize": True,
                    "acknowledged": None, "appoint_manager": True,
                    "goal": {"recipe": "wooden_bed", "minimum_stack_units": 6,
                             "max_order_amount": 100, "deadline_tick": tick + 300,
                             "poll_interval_ticks": 5, "stable_samples": 2, "assign_labor": True},
                }),
                "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN": "t" * 32,
                "DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN": "t" * 32,
            })
            retained_path = directory / "retained.json"
            try:
                child(executable, test, "control", case, environment, retained_path, retained_path)
            finally:
                # Restore only the bytes deliberately corrupted by this fault
                # injector after the actual owner has exited. Rust never repairs.
                if manager_peer.damaged_ack is not None:
                    (directory / "project/resource-project.ack").write_bytes(manager_peer.damaged_ack)
            retained = json.loads(retained_path.read_text())
            assert manager_peer.calls.count(2) == manager_peer.calls.count(3) == 1
            assert manager_peer.writer_count == 1 and 5 not in manager_peer.calls
            assert workshop_peer.writer_count == 0
            assert all(operation not in (2, 3, 5) for operation in workshop_peer.calls)
            assert all(not snapshot(directory / f"production/{name}") for name in ("parent", "workforce", "orders"))
            if case in ("lost_commit", "parent_ack", "handoff_ack"):
                assert not workshop_peer.calls, "failed Manager transfer touched the later child"
            else:
                assert workshop_peer.calls.count(1) == 1
                assert manager_peer.donor_at_export == snapshot(directory / "manager/custody"), "handoff changed its donor"
            if case in ("parent_ack", "handoff_ack"):
                assert manager_peer.damaged_ack is not None

            recovery_environment = reopened(environment, retained)
            offline = {key: value for key, value in recovery_environment.items()
                       if not (key.startswith("DFMCP_") and (key.endswith("_TOKEN")
                               or "_ALLOW_" in key and not key.startswith("DFMCP_ALLOW_UNADMITTED_")))}
            before_calls = (list(manager_peer.calls), list(workshop_peer.calls), list(endpoint.routes))
            before_files = custody(directory)
            phase = "reject_opening" if case == "stale_workshop" else "offline"
            child(executable, test, phase, case, offline, retained_path, directory / "offline.json")
            assert before_calls == (manager_peer.calls, workshop_peer.calls, endpoint.routes), "offline recovery contacted native source"
            assert before_files == custody(directory), "offline recovery rewrote custody"

            if case in ("lost_commit", "parent_ack", "handoff_ack"):
                manager_peer.drop_commit = False
                child(executable, test, "recover", case, recovery_environment,
                      retained_path, directory / "recovered.json")
                recovered = json.loads((directory / "recovered.json").read_text())
                current = recovered["reference"]["child_reference"]
                assert current["original"]["key"] == retained["original"]["key"]
                assert current["original"]["deadline_tick"] == tick + 300
                assert current["native_phase"] == 2 and not current["uncertain_effect"]
                assert manager_peer.calls.count(2) == manager_peer.calls.count(3) == 1
                assert manager_peer.writer_count == 1 and 5 not in manager_peer.calls
                assert not workshop_peer.calls
            elif case == "success":
                assert manager_peer.donor_at_export == snapshot(directory / "manager/custody")
        finally:
            try:
                endpoint.close()
            finally:
                try:
                    manager_peer.close()
                finally:
                    workshop_peer.close()
    print(f"resource-project-manager-case:{case}:checked")


def main():
    executable, test = sys.argv[1:]
    for case in ("success", "lost_commit", "parent_ack", "handoff_ack", "stale_workshop"):
        run_case(executable, test, case)
    print("resource-project-manager-campaign:checked")


if __name__ == "__main__":
    main()
