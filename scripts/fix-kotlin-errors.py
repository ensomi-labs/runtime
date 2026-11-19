"""Fix UniFFI 0.31's duplicate Throwable.message in this crate's generated Kotlin errors.

The TOML field rename also fails for types in Rust submodules in 0.31. Keep the Rust payload
as the override instead, leaving its name and the wire format unchanged. Fail if the template
changes, so a future UniFFI update must revisit this workaround.
"""

import sys
from pathlib import Path


def patch_exception(
    source: str,
    class_marker: str,
    converter_marker: str,
    expected_fields: int,
    getters: list[tuple[str, int]],
) -> str:
    start = source.index(class_marker)
    end = source.index(converter_marker, start)
    errors = source[start:end]
    field = "        val `message`: kotlin.String"
    if errors.count(field) != expected_fields:
        sys.exit(f"UniFFI error fields changed for {class_marker}")
    errors = errors.replace(field, "        override val `message`: kotlin.String")
    for expression, expected_count in getters:
        getter = "        override val message\n            get() = " + expression + "\n"
        if errors.count(getter) != expected_count:
            sys.exit(f"UniFFI error message template changed for {class_marker}")
        errors = errors.replace(getter, "")
    return source[:start] + errors + source[end:]


path = Path(sys.argv[1])
source = path.read_text()
source = patch_exception(
    source,
    "sealed class GeneratorException:",
    "public object FfiConverterTypeGeneratorError",
    expected_fields=2,
    getters=[
        ('"message=${ `message` }"', 1),
        ('"code=${ `code` }, message=${ `message` }"', 1),
    ],
)
if "sealed class AudioException:" in source:
    source = patch_exception(
        source,
        "sealed class AudioException:",
        "public object FfiConverterTypeAudioError",
        expected_fields=4,
        getters=[('"message=${ `message` }"', 4)],
    )
path.write_text(source)
