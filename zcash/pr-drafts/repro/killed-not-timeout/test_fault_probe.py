# Scratch reproduction (not for commit): with get_address.patch applied, the
# sentinel path 0xDEADBEEF makes the Tron sample panic while handling a request.
import pytest
from trezorlib.exceptions import TrezorFailure

from . import tron_ext


def test_faulted_app_is_reported(session, instance_id):
    with pytest.raises(TrezorFailure) as exc:
        tron_ext.get_address(session, instance_id, [0xDEADBEEF], show_display=False)
    print(f"OBSERVED: {exc.value}")
