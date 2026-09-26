"""Run sdk-wip's _extract_slip44_id from run.py on the host, unchanged.

Usage (from the repository root, on bieleluk/sdk-wip):
  python3 extract_slip44_id.py core/src/apps/extapp/run.py
"""

import ast
import sys


class DataError(Exception):
    pass


source = open(sys.argv[1]).read()
func = next(
    node
    for node in ast.parse(source).body
    if isinstance(node, ast.FunctionDef) and node.name == "_extract_slip44_id"
)
namespace = {"DataError": DataError}
exec(ast.get_source_segment(source, func), namespace)
extract = namespace["_extract_slip44_id"]

for patterns in (["m/44'/0'/account'"], ["m/44'/0'/account'", "m/44'/1'/account'"]):
    try:
        print(patterns, "->", extract(patterns))
    except DataError as e:
        print(patterns, "-> DataError:", e)
