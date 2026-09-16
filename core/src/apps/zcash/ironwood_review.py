"""Trusted review for a validated Ironwood synthetic PCZT."""


import ironwood_test
from trezor.enums import ButtonRequestType
from trezor.ui.layouts import confirm_action, confirm_properties, confirm_value

from .ironwood_account import require_session

def _format_amount(zatoshis: int) -> str:
    # trezor.strings.format_amount strips trailing zeroes; this flow needs eight.
    return f"{zatoshis // 100000000}.{zatoshis % 100000000:08d} test ZEC"


async def review_and_sign(review: dict, session_id: bytes) -> bytes:
    """Return signed PCZT bytes only after every trusted layout confirms."""
    try:
        await confirm_properties(
            br_name="ironwood_test_context",
            title="Synthetic review",
            subtitle="Synthetic data only",
            props=(
                ("Network", review["network"], False),
                ("Pool", review["pool"], False),
            ),
            br_code=ButtonRequestType.Other,
            verb="Continue",
        )

        for number, output in enumerate(review["outputs"], 1):
            kind = {
                "Payment": "Payment",
                "InternalChange": "Change",
            }[output["kind"]]
            await confirm_properties(
                br_name="ironwood_test_output",
                title=f"Test output {number}",
                props=(
                    ("Output type", kind, False),
                    ("Amount", _format_amount(output["value"]), False),
                ),
                verb="Continue",
            )
            # Use the complete-value layout, with no optional/skip-details intro.
            await confirm_value(
                br_name="ironwood_test_receiver",
                title="Ironwood receiver",
                subtitle=f"Test output {number}",
                description=f"{kind}\nRaw receiver bytes (hex)",
                value=output["receiver"].hex(),
                is_data=True,
                chunkify=False,
                br_code=ButtonRequestType.ConfirmOutput,
                verb="Continue",
            )

        await confirm_properties(
            br_name="ironwood_test_totals",
            title="Synthetic totals",
            subtitle=review["network"],
            props=(
                ("Total input", _format_amount(review["total_input"]), False),
                ("Payments", _format_amount(review["payments"]), False),
                ("Change", _format_amount(review["change"]), False),
                ("Fee", _format_amount(review["fee"]), False),
                ("Expires at block", str(review["expiry"]), False),
            ),
            br_code=ButtonRequestType.SignTx,
            verb="Continue",
        )
        # These helpers return None on confirmation and raise on cancellation.
        await confirm_action(
            br_name="ironwood_test_sign",
            title="Sign test transaction",
            action="Hold to sign this test transaction.",
            description=review["network"] + "\n" + review["pool"] + " synthetic test only.",
            verb="Hold to sign",
            verb_cancel="Cancel",
            hold=True,
            br_code=ButtonRequestType.SignTx,
        )
        require_session(session_id)
        return ironwood_test.sign(review["token"])
    finally:
        # Layout/sign failures cancel; the outer transport also covers begin failures.
        ironwood_test.cancel()
