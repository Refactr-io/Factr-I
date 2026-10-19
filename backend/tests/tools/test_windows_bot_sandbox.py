import pytest

from tools.windows_bot_sandbox import run, validate_plan


def plan():
    return {"command": r"C:\Factr-I\factr.exe", "args": ["--profile", "alpha", "serve", "--token-stdin"],
            "root": r"C:\Bots\alpha", "temp": r"C:\Temp\bots-local-a", "reads": [r"C:\Factr-I"]}


class FakeApi:
    def __init__(self, failure=None):
        self.calls = []
        self.failure = failure

    def call(self, name):
        self.calls.append(name)
        if self.failure == name:
            raise OSError("sandbox refused")

    def prepare(self, value):
        assert value == plan()
        self.call("restrict-token-acl-desktop-job")

    def create_suspended(self, value):
        self.call("create-suspended")

    def assign_job(self):
        self.call("assign-job")

    def resume(self):
        self.call("resume")

    def wait(self):
        self.call("wait-process-or-parent")
        return 7

    def close(self):
        self.call("close-job-and-handles")


def test_restrictions_and_tree_lifetime_precede_any_engine_work():
    api = FakeApi()
    assert run(plan(), api) == 7
    assert api.calls == ["restrict-token-acl-desktop-job", "create-suspended", "assign-job", "resume", "wait-process-or-parent", "close-job-and-handles"]


@pytest.mark.parametrize("failure", ["restrict-token-acl-desktop-job", "create-suspended", "assign-job", "resume", "wait-process-or-parent"])
def test_failure_closes_everything_and_never_starts_an_unrestricted_fallback(failure):
    api = FakeApi(failure)
    with pytest.raises(OSError, match="sandbox refused"):
        run(plan(), api)
    assert api.calls[-1] == "close-job-and-handles"
    if failure in ("restrict-token-acl-desktop-job", "create-suspended", "assign-job"):
        assert "resume" not in api.calls


@pytest.mark.parametrize("key,value", [("root", "relative"), ("command", "cmd\0.exe"), ("args", "serve"), ("reads", ["relative"]), ("extra", True)])
def test_untrusted_manifest_validation(key, value):
    value_plan = {**plan(), key: value}
    with pytest.raises(ValueError):
        validate_plan(value_plan)
