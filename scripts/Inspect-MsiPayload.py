"""Inspect a built MSI through read-only Windows Installer APIs and its CAB."""
import ctypes as ct
from ctypes import wintypes as wt
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

msi = ct.WinDLL("msi")
HANDLE = wt.UINT
msi.MsiOpenDatabaseW.argtypes = [wt.LPCWSTR, wt.LPCWSTR, ct.POINTER(HANDLE)]
msi.MsiDatabaseOpenViewW.argtypes = [HANDLE, wt.LPCWSTR, ct.POINTER(HANDLE)]
msi.MsiViewExecute.argtypes = [HANDLE, HANDLE]
msi.MsiViewFetch.argtypes = [HANDLE, ct.POINTER(HANDLE)]
msi.MsiRecordGetStringW.argtypes = [HANDLE, wt.UINT, wt.LPWSTR, ct.POINTER(wt.DWORD)]
msi.MsiRecordReadStream.argtypes = [HANDLE, wt.UINT, ct.c_void_p, ct.POINTER(wt.DWORD)]
msi.MsiCloseHandle.argtypes = [HANDLE]


def checked(code):
    if code:
        raise RuntimeError(f"Windows Installer read failed: {code}")


def field(record, index):
    length = wt.DWORD(0)
    code = msi.MsiRecordGetStringW(record, index, None, ct.byref(length))
    if code not in (0, 234):
        checked(code)
    buffer = ct.create_unicode_buffer(length.value + 1)
    length.value += 1
    checked(msi.MsiRecordGetStringW(record, index, buffer, ct.byref(length)))
    return buffer.value


def records(database, sql):
    view = HANDLE()
    checked(msi.MsiDatabaseOpenViewW(database, sql, ct.byref(view)))
    try:
        checked(msi.MsiViewExecute(view, 0))
        while True:
            record = HANDLE()
            code = msi.MsiViewFetch(view, ct.byref(record))
            if code == 259:
                return
            checked(code)
            try:
                yield record
            finally:
                msi.MsiCloseHandle(record)
    finally:
        msi.MsiCloseHandle(view)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    package = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2]).resolve()
    allowed = Path(__file__).resolve().parents[1] / "artifacts/native-0.1.1-20261015"
    output.relative_to(allowed.resolve())
    if output.exists() and any(output.iterdir()):
        raise RuntimeError("Payload output must be new or empty")
    output.mkdir(parents=True, exist_ok=True)
    database = HANDLE()
    checked(msi.MsiOpenDatabaseW(str(package), None, ct.byref(database)))
    try:
        keep = {"ProductCode", "ProductVersion", "ProductName", "UpgradeCode", "Manufacturer"}
        properties = {field(r, 1): field(r, 2) for r in records(database, "SELECT `Property`, `Value` FROM `Property`") if field(r, 1) in keep}
        files = [{"id": field(r, 1), "name": field(r, 2).split("|")[-1], "bytes": int(field(r, 3))} for r in records(database, "SELECT `File`, `FileName`, `FileSize` FROM `File`")]
        for record in records(database, "SELECT `Cabinet` FROM `Media`"):
            cabinet = field(record, 1)
            if not cabinet.startswith("#"):
                raise RuntimeError("Expected embedded cabinet")
            name = cabinet[1:]
            if not name or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-" for c in name):
                raise RuntimeError("Unsafe cabinet name")
            target = output / name
            with target.open("wb") as stream:
                for data_record in records(database, f"SELECT `Data` FROM `_Streams` WHERE `Name` = '{name}'"):
                    while True:
                        buffer = ct.create_string_buffer(1024 * 1024)
                        size = wt.DWORD(len(buffer))
                        checked(msi.MsiRecordReadStream(data_record, 1, buffer, ct.byref(size)))
                        if not size.value:
                            break
                        stream.write(buffer.raw[:size.value])
            subprocess.run([str(Path(__import__('os').environ['SystemRoot']) / 'System32/expand.exe'), '-F:*', str(target), str(output)], check=True, capture_output=True)
        for item in files:
            if Path(item["name"]).name != item["name"] or Path(item["id"]).name != item["id"]:
                raise RuntimeError("Unsafe payload file name")
            payload = output / item["id"]
            item["sha256"] = digest(payload)
            if payload.stat().st_size != item["bytes"]:
                raise RuntimeError("MSI payload size mismatch")
            shutil.copy2(payload, output / item["name"])
        result = {"package": str(package), "sha256": digest(package), "properties": properties, "files": files}
        (output / 'payload-manifest.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
        print(json.dumps(result, indent=2))
    finally:
        msi.MsiCloseHandle(database)


if __name__ == '__main__':
    main()
