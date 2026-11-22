"""Exercise the Unix updater with fake package managers; never installs an app."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
SHELL = r"C:\Program Files\Git\bin\bash.exe" if os.name == "nt" else "/bin/sh"


def shell_path(path):
    path = Path(path).resolve()
    if os.name == "nt":
        return "/" + path.drive[0].lower() + path.as_posix()[2:]
    return str(path)


def fake(name, args):
    def native(value):
        if os.name == "nt":
            value = value.replace("\\", "/")
            if len(value) > 3 and value[0] == "/" and value[2] == "/":
                value = value[1] + ":" + value[2:]
        return Path(value)
    with open(os.environ["TEST_TRACE"], "a", encoding="utf-8") as trace:
        trace.write(json.dumps([name, *args]) + "\n")
    if name in ("sha256sum", "shasum"):
        print(hashlib.sha256(native(args[-1]).read_bytes()).hexdigest() + "  " + args[-1])
    elif name == "id":
        print("1000")
    elif name == "pkexec":
        return subprocess.run([SHELL, "-c", 'exec "$@"', "sh", *args]).returncode
    elif name in ("apt-get", "pacman", "dnf"):
        return int(os.environ.get("TEST_FAIL_INSTALL", "0"))
    elif name == "hdiutil" and args[0] == "attach":
        app = native(args[args.index("-mountpoint") + 1]) / "PixClip.app/Contents"
        (app / "MacOS").mkdir(parents=True)
        (app / "MacOS/pixclip").write_text("new app")
        with (app / "Info.plist").open("wb") as output:
            plistlib.dump({"CFBundleIdentifier": "io.github.PeakNAtanon.PixClip"}, output)
    elif name == "hdiutil" and args[0] == "detach":
        app = native(args[1]) / "PixClip.app"
        assert app.resolve().is_relative_to(ROOT / ".cache")
        shutil.rmtree(app)
    elif name == "ditto":
        if os.environ.get("TEST_FAIL_COPY"):
            return 1
        shutil.copytree(native(args[0]), native(args[1]))
    elif name == "codesign":
        assert args[:3] == ["--verify", "--deep", "--strict"]
        return int(os.environ.get("TEST_BAD_SIGNATURE", "0"))
    elif name == "PlistBuddy":
        if os.environ.get("TEST_BAD_IDENTIFIER"):
            print("other.application")
        else:
            print(plistlib.loads(native(args[-1]).read_bytes())["CFBundleIdentifier"])
    elif name == "mv":
        source, destination = map(native, args[-2:])
        assert source.resolve().is_relative_to(ROOT / ".cache")
        assert destination.resolve().is_relative_to(ROOT / ".cache")
        if os.environ.get("TEST_FAIL_SWAP") and ".pixclip-new-" in Path(source).name:
            return 1
        shutil.move(source, destination)
    elif name == "osascript":
        body = sys.stdin.read()
        assert "quoted form" in body and "with administrator privileges" in body
        assert args[0] == "-"
        return subprocess.run([SHELL, args[1], "--replace-app", args[2], args[3]]).returncode
    return 0


def main():
    cache = ROOT / ".cache"
    cache.mkdir(exist_ok=True)
    original = (ROOT / "scripts/install-update.sh").read_text(encoding="utf-8")
    # Abort rather than accidentally calling real OS tools if the helper structure changes.
    assert original.count("PATH=/usr/bin:/bin:/usr/sbin:/sbin") == 1
    assert original.count("/usr/libexec/PlistBuddy") == 1
    assert original.count('if [ -w "$(dirname "$destination")" ] && {') == 1
    subprocess.run([SHELL, "-n", str(ROOT / "scripts/install-update.sh")], check=True)
    cases = [(kind, mode) for kind in ("deb", "rpm", "arch")
             for mode in ("success", "declined", "tampered")]
    cases += [("dmg", mode) for mode in
              ("success", "copy-failure", "bad-signature", "bad-identifier", "swap-failure", "admin")]
    with tempfile.TemporaryDirectory(prefix="unix-update-check-", dir=cache) as directory:
        base = Path(directory)
        assert base.resolve().is_relative_to(cache.resolve())
        for kind, mode in cases:
            folder = base / (kind + " " + mode + "'s")
            binaries = folder / "bin"
            binaries.mkdir(parents=True)
            trace = folder / "trace.txt"
            env = dict(os.environ, TEST_TRACE=str(trace))
            (folder / "tmp").mkdir()
            env["TMPDIR"] = shell_path(folder / "tmp")
            for variable in ("TEST_FAIL_INSTALL", "TEST_FAIL_COPY", "TEST_BAD_SIGNATURE",
                             "TEST_BAD_IDENTIFIER", "TEST_FAIL_SWAP", "TEST_ADMIN"):
                env.pop(variable, None)
            flags = {"declined": "TEST_FAIL_INSTALL", "copy-failure": "TEST_FAIL_COPY",
                     "bad-signature": "TEST_BAD_SIGNATURE", "bad-identifier": "TEST_BAD_IDENTIFIER",
                     "swap-failure": "TEST_FAIL_SWAP", "admin": "TEST_ADMIN"}
            if mode in flags:
                env[flags[mode]] = "1"
            for name in ("sha256sum", "shasum", "id", "pkexec", "apt-get", "pacman", "dnf",
                         "hdiutil", "ditto", "codesign", "PlistBuddy", "mv", "open", "osascript"):
                wrapper = binaries / name
                # Use native paths for native Python; the shell still quotes every argument.
                wrapper.write_text("#!/bin/sh\nexec " + shlex.quote(str(Path(sys.executable))) + " " +
                                   shlex.quote(str(Path(__file__).resolve())) + " --fake " + name + ' "$@"\n', encoding="utf-8", newline="\n")
                wrapper.chmod(0o755)
            helper = folder / "helper.sh"
            text = original.replace("PATH=/usr/bin:/bin:/usr/sbin:/sbin",
                                    "PATH=" + shlex.quote(shell_path(binaries)) + ":/usr/bin:/bin:/usr/sbin:/sbin")
            text = text.replace("/usr/libexec/PlistBuddy", shlex.quote(shell_path(binaries / "PlistBuddy")))
            text = text.replace('if [ -w "$(dirname "$destination")" ] && {',
                                'if [ "${TEST_ADMIN:-0}" = 0 ] && [ -w "$(dirname "$destination")" ] && {')
            helper.write_text(text, encoding="utf-8", newline="\n")
            package = folder / ("verified." + kind)
            package.write_bytes(b"verified installer fixture")
            digest = hashlib.sha256(package.read_bytes()).hexdigest()
            if mode == "tampered":
                digest = "0" * 64
            previous = folder / "previous"
            updated = folder / "updated"
            for executable, marker in ((previous, "restarted-old"), (updated, "restarted-new")):
                executable.write_text('#!/bin/sh\nprintf "' + marker + '\\n" >>"$TEST_TRACE"\n', newline="\n")
                executable.chmod(0o755)
            if kind == "dmg":
                updated = folder / "PixClip.app"
                (updated / "Contents/MacOS").mkdir(parents=True)
                (updated / "Contents/MacOS/pixclip").write_text("old app")
                previous = updated / "Contents/MacOS/pixclip"
            log = folder / "install.log"
            command = [SHELL, shell_path(helper), "999999999", kind, shell_path(package), digest,
                       shell_path(previous), shell_path(updated), shell_path(log)]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
            success = mode in ("success", "admin")
            assert (result.returncode == 0) == success, (kind, mode, result.stderr, log.read_text(errors="replace"))
            assert Path(str(log) + ".failed").exists() != success
            if kind == "dmg":
                assert (updated / "Contents/MacOS/pixclip").read_text() == ("new app" if success else "old app")
                assert not list(folder.glob("*.pixclip-old-*.app")), "Rollback must restore the old app"
            for _ in range(40):
                contents = trace.read_text(encoding="utf-8") if trace.exists() else ""
                if kind == "dmg" or "restarted-" in contents:
                    break
                time.sleep(0.05)
            if kind != "dmg":
                assert ("restarted-new" if success else "restarted-old") in contents
                if mode == "tampered":
                    assert "pkexec" not in contents, "Never request installation for a modified package"
            print(f"Unix updater: {kind}/{mode} passed (OS commands mocked)", flush=True)
        if sys.platform == "darwin":
            script = original.split("<<'APPLESCRIPT'\n", 1)[1].split("\nAPPLESCRIPT", 1)[0]
            source = base / "authorize.applescript"
            source.write_text(script)
            subprocess.run(["osacompile", "-o", str(base / "authorize.scpt"), str(source)], check=True)
            print("Native AppleScript syntax: passed", flush=True)


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "--fake":
        raise SystemExit(fake(sys.argv[2], sys.argv[3:]))
    main()
