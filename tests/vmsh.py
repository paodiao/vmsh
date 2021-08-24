#!/usr/bin/env python3

import subprocess
import threading
import os
from queue import Queue
from pathlib import Path
from nix import notos_image
from typing import Union, Any, Callable, List, Optional
from root import PROJECT_ROOT
from shlex import quote

EOF = 1


def cargo_build() -> Path:
    env = os.environ.copy()
    env["KERNELDIR"] = str(notos_image().kerneldir)
    if not os.environ.get("TEST_NO_REBUILD"):
        subprocess.run(["cargo", "build"], cwd=PROJECT_ROOT, env=env, check=True)
        subprocess.run(
            ["cargo", "build", "--examples"], cwd=PROJECT_ROOT, env=env, check=True
        )
    return PROJECT_ROOT.joinpath("target", "debug")


_build_artifacts: Optional[Path] = None


def build_artifacts() -> Path:
    global _build_artifacts
    if _build_artifacts is None:
        _build_artifacts = cargo_build()
    return _build_artifacts


class VmshPopen(subprocess.Popen):
    def process_stdout(self) -> None:
        self.lines: Queue[Union[str, int]] = Queue()
        threading.Thread(target=self.print_stdout).start()
        threading.Thread(target=self.print_stderr).start()

    def terminate(self) -> None:
        subprocess.run(["pkill", "--parent", str(self.pid)])

    def __exit__(self, exc_type: Any, exc_value: Any, traceback: Any) -> None:
        # we cannot kill sudo, but we can stop vmsh as it drops privileges to our user
        self.terminate()
        super().__exit__(exc_type, exc_value, traceback)

    def print_stdio_with_prefix(self, stdio: Any) -> None:
        buf = ""
        while True:
            assert stdio is not None
            res = stdio.read(1)

            if len(res) > 0:
                if res == "\n":
                    print(f"vmsh[{self.pid}] {buf}")
                    self.lines.put(buf)
                    buf = ""
                else:
                    buf += res
            else:
                if buf != "":
                    print(f"vmsh[{self.pid}] {buf}", flush=True)
                self.lines.put(EOF)
                return

    def print_stderr(self) -> None:
        self.print_stdio_with_prefix(self.stderr)

    def print_stdout(self) -> None:
        self.print_stdio_with_prefix(self.stdout)

    def wait_until_line(self, tag: str, condition: Callable[[str], bool]) -> None:
        """
        blocks until a line matching the given condition is printed
        Example: `vm.wait_until_line(lambda line: line == "foobar")`
        @param tag: printable, human readable tag
        """
        print(f"wait for '{tag}'...")
        while True:
            l = self.lines.get()

            if l == EOF:
                raise Exception("reach end of stdout output before process finished")

            if condition(str(l)):
                return


def spawn_vmsh_command(args: List[str], cargo_executable: str = "vmsh") -> VmshPopen:
    if not os.path.isdir("/sys/module/kheaders"):
        subprocess.run(["sudo", "modprobe", "kheaders"])
    uid = os.getuid()
    gid = os.getuid()
    groups = ",".join(map(str, os.getgroups()))
    cmd = [str(build_artifacts().joinpath(cargo_executable))]
    cmd += args
    cmd_quoted = " ".join(map(quote, cmd))

    cmd = [
        "sudo",
        "-E",
        "capsh",
        "--caps=cap_sys_ptrace,cap_dac_override,cap_sys_admin,cap_sys_resource+epi cap_setpcap,cap_setuid,cap_setgid+ep",
        "--keep=1",
        f"--groups={groups}",
        f"--gid={gid}",
        f"--uid={uid}",
        "--addamb=cap_sys_resource",
        "--addamb=cap_dac_override",
        "--addamb=cap_sys_admin",
        "--addamb=cap_sys_ptrace",
        "--",
        "-c",
        cmd_quoted,
    ]
    print("$ " + " ".join(map(quote, cmd)))
    p = VmshPopen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    p.process_stdout()
    return p
