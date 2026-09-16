"""Trusted seed acquisition shared by the isolated receive/sign workflows."""

import ironwood_test
from storage import cache
from trezor import utils
from trezor.wire import DataError, ProcessError, context

from apps.common import seed


def require_session(session_id: bytes) -> None:
    """Reject a wallet change before returning an address or using an approval."""
    seed.raise_if_not_initialized()
    if context.get_context().cache.export_session_id() != session_id:
        raise ProcessError("Wallet session changed")


async def get_seed() -> bytes:
    """Caller owns the exclusive workflow and borrows the result without an await."""
    if not utils.IRONWOOD_NATIVE_CALLER or utils.USE_THP:
        raise ProcessError("Unsupported synthetic test build")
    ironwood_test.cancel()
    try:
        seed.raise_if_not_initialized()
        session_id = context.get_context().cache.export_session_id()
        wallet_seed = await seed.get_seed()
        require_session(session_id)
        if type(wallet_seed) is not bytes or not 32 <= len(wallet_seed) <= 252:
            raise DataError("Unsupported wallet seed length")
        return wallet_seed
    except BaseException:
        # Legacy acquisition can cache after its passphrase await. If interrupted
        # or invalidated, none of those cached roots is safe to reuse. This clears
        # volatile sessions only, never wallet storage or backups.
        cache.clear_all()
        raise
