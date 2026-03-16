#!/usr/bin/env python3
"""Install the native messaging host manifest for Linux and Windows.

Usage:
  python install-host.py [path-to-flash-player-host-binary]

If no path is given, defaults to the binary in the workspace's
target/release directory.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import NoReturn

HOST_NAME = "org.cleanflash.flash_player"
HOST_DESCRIPTION = "Flash Player Native Messaging Host"
CHROME_EXTENSION_ID = "anhpdblalmhfhailclfnnlflfanhgohn"
FIREFOX_EXTENSION_ID = "flash-player@cleanflash.org"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Install native messaging manifests for the Flash Player extension."
    )
    parser.add_argument(
        "host_binary",
        nargs="?",
        help="Path to the flash-player-host binary. Defaults to ../target/release/flash-player-host(.exe).",
    )
    return parser.parse_args()


def default_host_binary(script_dir: Path) -> Path:
    binary_name = "flash-player-host.exe" if os.name == "nt" else "flash-player-host"
    return script_dir.parent / "target" / "release" / binary_name


def fail(message: str) -> NoReturn:
    print(f"Error: {message}", file=sys.stderr)
    raise SystemExit(1)


def resolve_host_binary(argument: str | None, script_dir: Path) -> Path:
    candidate = Path(argument).expanduser() if argument else default_host_binary(script_dir)
    candidate = candidate.resolve()

    if not candidate.exists() or candidate.is_dir():
        print(f"Error: host binary not found or invalid: {candidate}", file=sys.stderr)
        print("Build with: cargo build --release -p player-web", file=sys.stderr)
        raise SystemExit(1)

    if os.name != "nt" and not os.access(candidate, os.X_OK):
        fail(f"host binary is not executable: {candidate}")

    return candidate


def chrome_manifest(host_bin: Path) -> dict[str, object]:
    return {
        "name": HOST_NAME,
        "description": HOST_DESCRIPTION,
        "path": str(host_bin),
        "type": "stdio",
        "allowed_origins": [f"chrome-extension://{CHROME_EXTENSION_ID}/"],
    }


def firefox_manifest(host_bin: Path) -> dict[str, object]:
    return {
        "name": HOST_NAME,
        "description": HOST_DESCRIPTION,
        "path": str(host_bin),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID],
    }


def write_manifest(path: Path, manifest: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")


def install_linux(host_bin: Path) -> None:
    home = Path.home()

    firefox_path = home / ".mozilla" / "native-messaging-hosts" / f"{HOST_NAME}.json"
    write_manifest(firefox_path, firefox_manifest(host_bin))
    print(f"Installed Firefox manifest: {firefox_path}")

    chrome_targets = [
        ("Chrome", home / ".config" / "google-chrome" / "NativeMessagingHosts"),
        ("Chromium", home / ".config" / "chromium" / "NativeMessagingHosts"),
        (
            "Brave",
            home
            / ".config"
            / "BraveSoftware"
            / "Brave-Browser"
            / "NativeMessagingHosts",
        ),
    ]

    for browser_name, directory in chrome_targets:
        manifest_path = directory / f"{HOST_NAME}.json"
        write_manifest(manifest_path, chrome_manifest(host_bin))
        print(f"Installed {browser_name} manifest: {manifest_path}")


def register_windows_host(browser_key: str, manifest_path: Path) -> None:
    import winreg

    key_path = f"{browser_key}\\{HOST_NAME}"
    with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER, key_path, 0, winreg.KEY_WRITE) as key:
        winreg.SetValueEx(key, None, 0, winreg.REG_SZ, str(manifest_path))
    print(f"Registered {key_path} -> {manifest_path}")


def install_windows(host_bin: Path) -> None:
    local_appdata = Path(
        os.environ.get("LOCALAPPDATA", str(Path.home() / "AppData" / "Local"))
    )
    manifest_dir = local_appdata / "Flash Player" / "NativeMessagingHosts"

    chrome_path = manifest_dir / f"{HOST_NAME}.chrome.json"
    firefox_path = manifest_dir / f"{HOST_NAME}.firefox.json"

    write_manifest(chrome_path, chrome_manifest(host_bin))
    write_manifest(firefox_path, firefox_manifest(host_bin))

    print(f"Installed Chrome-family manifest: {chrome_path}")
    print(f"Installed Firefox manifest: {firefox_path}")

    register_windows_host(r"Software\Google\Chrome\NativeMessagingHosts", chrome_path)
    register_windows_host(r"Software\Chromium\NativeMessagingHosts", chrome_path)
    register_windows_host(
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        chrome_path,
    )
    register_windows_host(r"Software\Mozilla\NativeMessagingHosts", firefox_path)


def plugin_hint() -> str:
    return "PepperFlash .dll path" if os.name == "nt" else "PepperFlash .so path"


def main() -> int:
    args = parse_args()
    script_dir = Path(__file__).resolve().parent
    host_bin = resolve_host_binary(args.host_binary, script_dir)

    try:
        if os.name == "nt":
            install_windows(host_bin)
        elif sys.platform.startswith("linux"):
            install_linux(host_bin)
        else:
            fail(f"unsupported platform: {sys.platform}")
    except OSError as exc:
        fail(str(exc))

    print()
    print(f"Done. Make sure FLASH_PLUGIN_PATH is set to the {plugin_hint()}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())