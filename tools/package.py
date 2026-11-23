#!/usr/bin/env python3
"""Build, check, and copy the native Horizon package into dist/."""

import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
MOD = ROOT / "hmod"


def run(*command: str) -> None:
    subprocess.run(command, cwd=MOD, check=True, stdin=subprocess.DEVNULL)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launcher", default=shutil.which("horizon-launcher"))
    args = parser.parse_args()
    if not args.launcher:
        parser.error("horizon-launcher is not on PATH; supply --launcher")

    manifest = tomllib.loads((MOD / "horizon-mod.toml").read_text(encoding="utf-8"))
    mod_id, version = manifest["id"], manifest["version"]
    package = MOD / "target" / "horizon-mod" / f"{mod_id}.hmod"
    output = ROOT / "dist" / f"litematica-horizon-{version}.hmod"

    run(str(Path(args.launcher).resolve()), "native-lock", ".", "--update-lock")
    run("cargo", "horizon-mod", "build")
    run("cargo", "horizon-mod", "check", str(package))
    output.parent.mkdir(exist_ok=True)
    shutil.copyfile(package, output)
    with output.open("rb") as archive:
        digest = hashlib.file_digest(archive, "sha256").hexdigest()
    print(f"{output}\nSHA-256 {digest}")


if __name__ == "__main__":
    main()
