"""Fix UniFFI 0.31's duplicate Throwable.message in this crate's generated Kotlin errors.

The TOML field rename also fails for types in Rust submodules in 0.31. Keep the Rust payload
as the override instead, leaving its name and the wire format unchanged. Fail if the template
changes, so a future UniFFI update must revisit this workaround.
"""

import sys
from pathlib import Path

path = Path(sys.argv[1])
source = path.read_text()
start = source.index("sealed class GeneratorException:")
end = source.index("public object FfiConverterTypeGeneratorError", start)
errors = source[start:end]
field = "        val `message`: kotlin.String"
if errors.count(field) != 2:
    sys.exit("UniFFI error fields changed")
errors = errors.replace(field, "        override val `message`: kotlin.String")
for expression in ['"message=${ `message` }"', '"code=${ `code` }, message=${ `message` }"']:
    getter = "        override val message\n            get() = " + expression + "\n"
    if errors.count(getter) != 1:
        sys.exit("UniFFI error message template changed")
    errors = errors.replace(getter, "")
path.write_text(source[:start] + errors + source[end:])
