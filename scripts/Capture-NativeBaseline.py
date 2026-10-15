"""Read-only metadata baseline; never saves user configuration contents."""
import hashlib
import json
import os
from pathlib import Path
import sys
import winreg


def file_metadata(path):
    if not path.is_file():
        return {"exists": False}
    stat = path.stat()
    return {"exists": True, "bytes": stat.st_size, "mtime_ns": stat.st_mtime_ns,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def snapshot():
    user = Path(os.environ["USERPROFILE"])
    names = ["index.db", "index.db.lock", "index.db-wal", "index.db-shm",
             "index.db-journal", "config.json", "config.json.lock", "prefs.db",
             "prefs.db-wal", "prefs.db-shm", "prefs.db-journal"]
    installs = []
    for hive_name, hive in [("HKCU", winreg.HKEY_CURRENT_USER), ("HKLM", winreg.HKEY_LOCAL_MACHINE)]:
        for view in [winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY]:
            try:
                root = winreg.OpenKey(hive, r"Software\Microsoft\Windows\CurrentVersion\Uninstall", 0, winreg.KEY_READ | view)
            except FileNotFoundError:
                continue
            with root:
                for index in range(winreg.QueryInfoKey(root)[0]):
                    name = winreg.EnumKey(root, index)
                    try:
                        with winreg.OpenKey(root, name) as key:
                            display = winreg.QueryValueEx(key, "DisplayName")[0]
                            if "SessionAtlas" not in str(display):
                                continue
                            values = {}
                            for field in ["DisplayName", "DisplayVersion", "InstallLocation", "UninstallString"]:
                                try:
                                    values[field] = winreg.QueryValueEx(key, field)[0]
                                except FileNotFoundError:
                                    pass
                            installs.append({"hive": hive_name, "view": view, "key": name, "values": values})
                    except FileNotFoundError:
                        pass
    shortcut_roots = [user / "Desktop", Path(os.environ["PUBLIC"]) / "Desktop",
                      Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs",
                      Path(os.environ["PROGRAMDATA"]) / "Microsoft/Windows/Start Menu/Programs"]
    shortcuts = {}
    for root in shortcut_roots:
        if root.exists():
            for path in root.rglob("*SessionAtlas*.lnk"):
                shortcuts[str(path)] = file_metadata(path)
    return {"files": {name: file_metadata(user / ".sessionatlas" / name) for name in names},
            "uninstall": installs, "shortcuts": shortcuts}


if __name__ == "__main__":
    target = Path(sys.argv[1]).resolve()
    target.write_text(json.dumps(snapshot(), ensure_ascii=False, indent=2), encoding="utf-8")
    print("Captured metadata baseline:", target)
