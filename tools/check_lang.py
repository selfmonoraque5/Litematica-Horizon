"""Checks the mod's lang files against the code and each other.

- every key the code names with `tr!("...")` is in en_us.json (keys built at run time are listed
  in DYNAMIC below);
- every language has exactly en_us's keys;
- each text has the same placeholders as the English one (`%s` count, `%n$s` positions).

Run from anywhere: python tools/check_lang.py
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "hmod"
LANG = ROOT / "assets" / "blazematica" / "lang"
PREFIX = "blazematica."

# Keys the code builds with format!(...) instead of naming them.
DYNAMIC = (
    [f"help.{i}" for i in range(1, 16)]
    + [f"credits.{i}" for i in range(1, 7)]
    + [f"menu.tool.help_{i}" for i in range(1, 5)]
    + [f"rotation.{r}" for r in ("none", "cw_90", "180", "ccw_90")]
    + [f"mirror.{m}" for m in ("none", "left_right", "front_back")]
    + [f"compass.{c}" for c in ("e", "se", "s", "sw", "w", "nw", "n", "ne")]
    + [f"menu.tab.{t}" for t in ("load", "placement", "save", "tool", "printer", "display", "materials")]
)


def placeholders(text: str) -> tuple:
    text = text.replace("%%", "")
    positional = sorted(re.findall(r"%(\d+)\$s", text))
    plain = len(re.findall(r"%s", text))
    return plain, tuple(positional)


def main() -> int:
    problems = []
    en = json.loads((LANG / "en_us.json").read_text(encoding="utf-8"))
    short = {k[len(PREFIX):] for k in en if k.startswith(PREFIX)}

    used = set(DYNAMIC)
    for path in (ROOT / "src").rglob("*.rs"):
        if path.name == "i18n.rs":  # the macro itself; its docs show made-up keys
            continue
        used |= set(re.findall(r'tr!\(\s*"([^"]+)"', path.read_text(encoding="utf-8")))
    for key in sorted(used - short):
        problems.append(f"en_us.json lacks {PREFIX}{key} (used in the code)")
    # setting.* and key.* are named by the game (registered settings and keys), not by tr!.
    for key in sorted(short - used):
        if not key.startswith(("setting.", "key.")) and key not in ("on", "off"):
            problems.append(f"en_us.json has {PREFIX}{key}, which the code doesn't use")

    for path in sorted(LANG.glob("*.json")):
        if path.name == "en_us.json":
            continue
        other = json.loads(path.read_text(encoding="utf-8"))
        for key in sorted(set(en) - set(other)):
            problems.append(f"{path.name} lacks {key}")
        for key in sorted(set(other) - set(en)):
            problems.append(f"{path.name} has {key}, which en_us.json doesn't")
        for key in sorted(set(en) & set(other)):
            if placeholders(en[key]) != placeholders(other[key]):
                problems.append(f"{path.name}: {key} has other placeholders than English: {other[key]!r}")

    for p in problems:
        print(p)
    print(f"{len(en)} keys, {len(list(LANG.glob('*.json')))} languages, {len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
