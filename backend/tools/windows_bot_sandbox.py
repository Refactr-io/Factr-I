"""Windows local bot launcher: restricted low-integrity token, private desktop and kill-on-close job.

The Desktop passes only paths/argv, never credentials. The engine inherits stdin for --token-stdin.
This is a local sandbox: public system files and the network remain accessible. Private profile ACLs
distinguish bots, and the user's SID is deny-only. See computer/LOCAL-SANDBOX.md for the boundary.
"""
from __future__ import annotations

import hashlib
import json
import ntpath
import os
import subprocess
import sys


def validate_plan(value):
    if not isinstance(value, dict) or set(value) != {"command", "args", "root", "temp", "reads"}:
        raise ValueError("Invalid local sandbox launch plan")
    for key in ("command", "root", "temp"):
        if not isinstance(value[key], str) or not ntpath.isabs(value[key]) or "\0" in value[key]:
            raise ValueError("Sandbox paths must be absolute")
    for key in ("args", "reads"):
        if not isinstance(value[key], list) or any(not isinstance(p, str) or "\0" in p for p in value[key]):
            raise ValueError("Invalid sandbox arguments")
    if any(not ntpath.isabs(p) for p in value["reads"]):
        raise ValueError("Runtime paths must be absolute")
    return value


def run(plan, api):
    """Never resume a child until every restriction is applied; any failure closes the job."""
    plan = validate_plan(plan)
    try:
        api.prepare(plan)
        api.create_suspended(plan)
        api.assign_job()
        api.resume()
        return api.wait()
    finally:
        api.close()


class WindowsApi:
    def __init__(self):
        import ctypes as c
        from ctypes import wintypes as w
        self.c, self.w = c, w
        self.k = c.WinDLL("kernel32", use_last_error=True)
        self.a = c.WinDLL("advapi32", use_last_error=True)
        self.u = c.WinDLL("user32", use_last_error=True)
        self.handles = []
        self.memory = []
        self.process = self.thread = self.job = self.desktop = None

        class Sid(c.Structure):
            _fields_ = [("sid", c.c_void_p), ("attributes", w.DWORD)]
        class Startup(c.Structure):
            _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR), ("title", w.LPWSTR),
                        *[(n, w.DWORD) for n in ("x", "y", "xsize", "ysize", "xchars", "ychars", "fill", "flags")],
                        ("show", w.WORD), ("reserved_size", w.WORD), ("reserved_ptr", c.c_void_p),
                        ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]
        class Process(c.Structure):
            _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE), ("pid", w.DWORD), ("tid", w.DWORD)]
        class BasicLimit(c.Structure):
            _fields_ = [("process_time", c.c_int64), ("job_time", c.c_int64), ("flags", w.DWORD),
                        ("min_ws", c.c_size_t), ("max_ws", c.c_size_t), ("active", w.DWORD),
                        ("affinity", c.c_size_t), ("priority", w.DWORD), ("scheduling", w.DWORD)]
        class JobLimit(c.Structure):
            _fields_ = [("basic", BasicLimit), ("io", c.c_uint64 * 6),
                        *[(n, c.c_size_t) for n in ("process_mem", "job_mem", "peak_process_mem", "peak_job_mem")]]
        class Security(c.Structure):
            _fields_ = [("length", w.DWORD), ("descriptor", c.c_void_p), ("inherit", w.BOOL)]
        self.Sid, self.Startup, self.Process, self.JobLimit, self.Security = Sid, Startup, Process, JobLimit, Security
        p = c.c_void_p
        signatures = [
            (self.k, "GetCurrentProcess", [], w.HANDLE),
            (self.k, "CloseHandle", [w.HANDLE], w.BOOL),
            (self.k, "LocalFree", [p], p),
            (self.k, "CreateJobObjectW", [p, w.LPCWSTR], w.HANDLE),
            (self.k, "SetInformationJobObject", [w.HANDLE, c.c_int, p, w.DWORD], w.BOOL),
            (self.k, "AssignProcessToJobObject", [w.HANDLE, w.HANDLE], w.BOOL),
            (self.k, "ResumeThread", [w.HANDLE], w.DWORD),
            (self.k, "TerminateProcess", [w.HANDLE, w.UINT], w.BOOL),
            (self.k, "OpenProcess", [w.DWORD, w.BOOL, w.DWORD], w.HANDLE),
            (self.k, "WaitForMultipleObjects", [w.DWORD, p, w.BOOL, w.DWORD], w.DWORD),
            (self.k, "GetExitCodeProcess", [w.HANDLE, p], w.BOOL),
            (self.k, "GetStdHandle", [w.DWORD], w.HANDLE),
            (self.k, "SetHandleInformation", [w.HANDLE, w.DWORD, w.DWORD], w.BOOL),
            (self.a, "OpenProcessToken", [w.HANDLE, w.DWORD, p], w.BOOL),
            (self.a, "GetTokenInformation", [w.HANDLE, c.c_int, p, w.DWORD, p], w.BOOL),
            (self.a, "SetTokenInformation", [w.HANDLE, c.c_int, p, w.DWORD], w.BOOL),
            (self.a, "GetLengthSid", [p], w.DWORD),
            (self.a, "ConvertSidToStringSidW", [p, p], w.BOOL),
            (self.a, "ConvertStringSidToSidW", [w.LPCWSTR, p], w.BOOL),
            (self.a, "ConvertStringSecurityDescriptorToSecurityDescriptorW", [w.LPCWSTR, w.DWORD, p, p], w.BOOL),
            (self.a, "GetSecurityDescriptorDacl", [p, p, p, p], w.BOOL),
            (self.a, "GetSecurityDescriptorSacl", [p, p, p, p], w.BOOL),
            (self.a, "SetNamedSecurityInfoW", [w.LPWSTR, c.c_int, w.DWORD, p, p, p, p], w.DWORD),
            (self.a, "CreateRestrictedToken", [w.HANDLE, w.DWORD, w.DWORD, p, w.DWORD, p, w.DWORD, p, p], w.BOOL),
            (self.a, "CreateProcessAsUserW", [w.HANDLE, w.LPCWSTR, w.LPWSTR, p, p, w.BOOL, w.DWORD, p, w.LPCWSTR, p, p], w.BOOL),
            (self.u, "CreateDesktopW", [w.LPCWSTR, p, p, w.DWORD, w.DWORD, p], w.HANDLE),
            (self.u, "CloseDesktop", [w.HANDLE], w.BOOL),
        ]
        for dll, name, args, result in signatures:
            fn = getattr(dll, name)
            fn.argtypes, fn.restype = args, result

    def check(self, result):
        if not result:
            raise self.c.WinError(self.c.get_last_error())
        return result

    def sid(self, text):
        value = self.c.c_void_p()
        self.check(self.a.ConvertStringSidToSidW(text, self.c.byref(value)))
        self.memory.append(value)
        return value

    def sid_text(self, sid):
        value = self.w.LPWSTR()
        self.check(self.a.ConvertSidToStringSidW(sid, self.c.byref(value)))
        try:
            return value.value
        finally:
            self.k.LocalFree(value)

    def info(self, token, kind):
        size = self.w.DWORD()
        self.a.GetTokenInformation(token, kind, None, 0, self.c.byref(size))
        buffer = self.c.create_string_buffer(size.value)
        self.check(self.a.GetTokenInformation(token, kind, buffer, size, self.c.byref(size)))
        return buffer

    def descriptor(self, sddl):
        value = self.c.c_void_p()
        self.check(self.a.ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, 1, self.c.byref(value), None))
        self.memory.append(value)
        return value

    def protect(self, filename, descriptor):
        c, w = self.c, self.w
        present, defaulted = w.BOOL(), w.BOOL()
        dacl, sacl = c.c_void_p(), c.c_void_p()
        self.check(self.a.GetSecurityDescriptorDacl(descriptor, c.byref(present), c.byref(dacl), c.byref(defaulted)))
        self.check(self.a.GetSecurityDescriptorSacl(descriptor, c.byref(present), c.byref(sacl), c.byref(defaulted)))
        error = self.a.SetNamedSecurityInfoW(filename, 1, 0x80000014, None, None, dacl, sacl)
        if error:
            raise c.WinError(error)

    def prepare(self, plan):
        c, w = self.c, self.w
        original = w.HANDLE()
        self.check(self.a.OpenProcessToken(self.k.GetCurrentProcess(), 0x000B, c.byref(original)))
        self.handles.append(original)
        # Copy the SID string before the token-information buffer goes out of scope.
        user_info = self.info(original, 1)
        self.user = self.sid_text(self.Sid.from_buffer(user_info).sid)
        groups = self.info(original, 2)
        count = w.DWORD.from_buffer(groups).value
        offset = (c.sizeof(w.DWORD) + c.alignment(self.Sid) - 1) // c.alignment(self.Sid) * c.alignment(self.Sid)
        group_rows = (self.Sid * count).from_buffer(groups, offset)
        logon = next((self.sid_text(row.sid) for row in group_rows if row.attributes & 0xC0000000 == 0xC0000000), None)
        if not logon:
            raise RuntimeError("The local sandbox requires an interactive Windows login")
        digest = hashlib.sha256(ntpath.normcase(plan["root"]).encode()).digest()
        self.bot_sid = "S-1-5-21-" + "-".join(str(int.from_bytes(digest[i:i + 4], "little") & 0x7FFFFFFF) for i in range(0, 16, 4))
        disabled = (self.Sid * 3)(*[self.Sid(self.sid(s), 0) for s in (self.user, "S-1-5-32-544", "S-1-5-32-547")])
        restricted = (self.Sid * 4)(*[self.Sid(self.sid(s), 0) for s in (self.bot_sid, "S-1-1-0", "S-1-5-32-545", "S-1-5-11")])
        self.token = w.HANDLE()
        self.check(self.a.CreateRestrictedToken(original, 1, len(disabled), disabled, 0, None, len(restricted), restricted, c.byref(self.token)))
        self.handles.append(self.token)
        low = self.Sid(self.sid("S-1-16-4096"), 0x20)
        self.check(self.a.SetTokenInformation(self.token, 25, c.byref(low), c.sizeof(low) + self.a.GetLengthSid(low.sid)))
        # Each bot's protected ACL has no public/group grant. Another bot's restricted SID cannot pass it.
        access = f"(A;OICI;RC;;;OW)(A;OICI;FA;;;{self.user})(A;OICI;FA;;;{logon})(A;OICI;FA;;;{self.bot_sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
        descriptor = self.descriptor("D:P" + access + "S:(ML;OICI;NW;;;LW)")
        for root in (plan["root"], plan["temp"]):
            self.protect(root, descriptor)
            for directory, dirs, files in os.walk(root, followlinks=False):
                dirs[:] = [p for p in dirs if not os.path.islink(os.path.join(directory, p)) and not os.path.isjunction(os.path.join(directory, p))]
                for name in dirs + files:
                    filename = os.path.join(directory, name)
                    if not os.path.islink(filename):
                        self.protect(filename, descriptor)
        # A per-user portable app needs explicit runtime read access because the user's SID is deny-only.
        user_home = os.environ.get("HOMEDRIVE", "") + os.environ.get("HOMEPATH", "")
        for runtime in plan["reads"]:
            try:
                portable = bool(user_home) and ntpath.commonpath([user_home, runtime]).lower() == user_home.lower()
            except ValueError:
                portable = False
            if portable:
                subprocess.run(["icacls.exe", runtime, "/grant:r", f"*{logon}:(OI)(CI)RX", f"*{self.bot_sid}:(OI)(CI)RX", "/T", "/L"], check=True, capture_output=True)
        desktop_name = "bots-local-" + hashlib.sha256(plan["root"].encode()).hexdigest()[:20]
        desktop_sd = self.descriptor(f"D:P(A;;GA;;;{self.user})(A;;GA;;;{logon})(A;;GA;;;{self.bot_sid})S:(ML;;NW;;;LW)")
        security = self.Security(c.sizeof(self.Security), desktop_sd, False)
        self.desktop = self.check(self.u.CreateDesktopW(desktop_name, None, None, 0, 0x01FF, c.byref(security)))
        self.desktop_name = desktop_name
        self.job = self.check(self.k.CreateJobObjectW(None, None))
        limits = self.JobLimit()
        limits.basic.flags = 0x2000  # KILL_ON_JOB_CLOSE, deliberately no breakaway flags.
        self.check(self.k.SetInformationJobObject(self.job, 9, c.byref(limits), c.sizeof(limits)))
        ui = w.DWORD(0xFF)  # No host clipboard, desktop switching, outside UI handles or exit-Windows access.
        self.check(self.k.SetInformationJobObject(self.job, 4, c.byref(ui), c.sizeof(ui)))
        self.parent = self.check(self.k.OpenProcess(0x00100000, False, os.getppid()))
        self.handles.append(self.parent)

    def create_suspended(self, plan):
        c, w = self.c, self.w
        startup = self.Startup()
        startup.cb, startup.flags, startup.desktop = c.sizeof(startup), 0x100, self.desktop_name
        startup.stdin, startup.stdout, startup.stderr = [self.k.GetStdHandle(i & 0xFFFFFFFF) for i in (-10, -11, -12)]
        for handle in (startup.stdin, startup.stdout, startup.stderr):
            self.check(self.k.SetHandleInformation(handle, 1, 1))
        env = dict(os.environ)
        # The unrestricted launcher holds a real parent handle. The restricted child cannot inspect Desktop.
        for key in ("FACTR_PARENT_PID", "FACTR_PARENT_START_MARKER", "FACTR_PARENT_NONCE"):
            env.pop(key, None)
        environment = c.create_unicode_buffer("\0".join(f"{key}={value}" for key, value in sorted(env.items(), key=lambda p: p[0].upper())) + "\0\0")
        info = self.Process()
        command = c.create_unicode_buffer(subprocess.list2cmdline([plan["command"], *plan["args"]]))
        self.check(self.a.CreateProcessAsUserW(self.token, plan["command"], command, None, None, True,
                                             0x08000404, environment, os.getcwd(), c.byref(startup), c.byref(info)))
        self.process, self.thread = info.process, info.thread

    def assign_job(self):
        self.check(self.k.AssignProcessToJobObject(self.job, self.process))

    def resume(self):
        if self.k.ResumeThread(self.thread) == 0xFFFFFFFF:
            self.check(False)

    def wait(self):
        handles = (self.w.HANDLE * 2)(self.process, self.parent)
        result = self.k.WaitForMultipleObjects(2, handles, False, 0xFFFFFFFF)
        if result == 1:
            return 1  # Parent died; closing the job kills the engine and every descendant.
        if result != 0:
            self.check(False)
        code = self.w.DWORD()
        self.check(self.k.GetExitCodeProcess(self.process, self.c.byref(code)))
        return code.value

    def close(self):
        if self.process:
            self.k.TerminateProcess(self.process, 1)  # Also covers failure before job assignment.
        if self.job:
            self.k.CloseHandle(self.job)
        for handle in (self.thread, self.process, *reversed(self.handles)):
            if handle:
                self.k.CloseHandle(handle)
        if self.desktop:
            self.u.CloseDesktop(self.desktop)
        for pointer in self.memory:
            self.k.LocalFree(pointer)


if __name__ == "__main__":
    try:
        if os.name != "nt":
            raise RuntimeError("This launcher requires Windows")
        sys.exit(run(json.loads(sys.argv[1]), WindowsApi()))
    except Exception as error:
        print(f"local-sandbox-unavailable: {error}. Set up Docker or ask your administrator to allow the local sandbox.", file=sys.stderr)
        sys.exit(1)
