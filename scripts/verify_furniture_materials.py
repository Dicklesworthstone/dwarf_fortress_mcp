#!/usr/bin/env python3
"""Match an independent DFHack material observation to selected operations arrivals."""

import argparse
import hashlib
import json
import re
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, minimum, maximum):
    return type(value) is int and minimum <= value <= maximum


def load(path):
    with path.open("rb") as stream:
        data = stream.read(2 * 1024 * 1024 + 1)
    require(len(data) <= 2 * 1024 * 1024, "input exceeds 2 MiB")
    return json.loads(data)


def selected(arrivals):
    require(arrivals["ok"] is True and arrivals["kind"] == "historical_furniture_arrivals",
            "expected a successful decoded historical_furniture_arrivals response")
    source = arrivals["target_source"]
    require(source["paused"] is True, "target was unpaused; capture a paused endpoint")
    rows = arrivals["new_item_examples"]
    count = arrivals["new_item_count"]
    skipped = arrivals.get("examples_skipped", 0)
    require(integer(count, 1, 65536) and integer(skipped, 0, count) and isinstance(rows, list)
            and len(rows) == min(count - skipped, 8) and arrivals["examples_truncated"] is (count > len(rows)),
            "inconsistent arrival example page")
    require(rows, "this page has no selected furniture examples")
    before, after = arrivals["before_next_item_id"], arrivals["after_next_item_id"]
    require(integer(before, 0, 2**31-1) and integer(after, before, 2**31-1), "invalid item frontier")
    floor = arrivals.get("examples_from_item_id", before)
    require(integer(floor, before, 2**31-1), "invalid item example floor")
    ids = [row["native_item_id"] for row in rows]
    require(all(integer(id_, floor, after-1) for id_ in ids) and ids == sorted(set(ids)),
            "invalid or duplicate selected item IDs")
    require(arrivals["native_type_key"] in ("BED", "CHAIR", "DOOR", "TABLE"), "invalid furniture type")
    target = arrivals["comparison"]["to"]
    require(target["anchor"] == arrivals["anchor"] and re.fullmatch("[0-9a-f]{64}", target["record_digest"]),
            "inconsistent target record")
    return rows


def compare(arrivals, observation):
    examples = selected(arrivals)
    require(observation["schema"] == "dfmcp.furniture_material_observation/1", "unexpected observation schema")
    source = arrivals["target_source"]
    require(observation["paused"] is True and all(
        observation[key] == source[key] for key in ("df_version", "dfhack_version")
    ), "pause state or software version differs")
    folder = observation["world_folder"].encode("utf-8")
    site = observation["site_id"]
    require(0 < len(folder) <= 512 and b"\0" not in folder and integer(site, 0, 2**31-1),
            "invalid native fortress identity")
    data = b"dfmcp-live-fortress-id-v1\0" + folder + b"\0" + site.to_bytes(4, "big")
    fortress = str(int.from_bytes(hashlib.sha256(data).digest()[:8], "big") | 1)
    anchor = arrivals["anchor"]
    require(fortress == anchor["fortress_id"], "fortress differs")
    year, tick = observation["year"], observation["year_tick"]
    require(integer(year, 0, 2**32-1) and integer(tick, 0, 403199)
            and year * 403200 + tick == anchor["game_tick"], "observation clock differs")
    frontier = observation["next_item_id"]
    require(integer(frontier, 0, 2**31-1) and frontier == arrivals["after_next_item_id"], "item frontier differs")
    rows = observation["items"]
    require(isinstance(rows, list) and len(rows) == len(examples), "selected item coverage differs")
    results = []
    for example, row in zip(examples, rows, strict=True):
        pair = row["material"]
        require(isinstance(pair, list) and len(pair) == 2 and all(integer(v, -1, 2**31-1) for v in pair)
                and integer(row["native_item_id"], 0, 2**31-1) and integer(row["subtype"], -1, 2**31-1)
                and all(row[key] == example[key] for key in ("native_item_id", "subtype", "material"))
                and row["type_key"] == arrivals["native_type_key"], "selected item identity or properties differ")
        result = {"native_item_id": row["native_item_id"], "material": pair, "classification": "unknown"}
        require(type(row["resolved"]) is bool, "invalid material resolution")
        if row["resolved"]:
            mode, flag, token = row["mode"], row["wood_flag"], row["token"]
            require(mode in ("builtin", "inorganic", "plant", "creature") and type(flag) is bool
                    and isinstance(token, str) and 0 < len(token.encode()) <= 512
                    and not any(ord(char) < 32 or ord(char) == 127 for char in token),
                    "invalid decoded material")
            result.update(classification="plant_wood" if mode == "plant" and flag else "not_plant_wood",
                          mode=mode, wood_flag=flag, token=token)
        results.append(result)
    return {
        "schema": "dfmcp.furniture_material_comparison/1",
        "target_record_digest": arrivals["comparison"]["to"]["record_digest"],
        "target_anchor": anchor,
        "evidence_scope": "independent_paused_read_of_selected_ids",
        "new_item_count": arrivals["new_item_count"], "examples_truncated": arrivals["examples_truncated"],
        "selected_plant_wood_count": sum(row["classification"] == "plant_wood" for row in results),
        "selected_unknown_material_count": sum(row["classification"] == "unknown" for row in results),
        "examples_from_item_id": arrivals.get("examples_from_item_id", arrivals["before_next_item_id"]),
        "examples_skipped": arrivals.get("examples_skipped", 0),
        "items": results, "same_process_incarnation_proven": False, "order_attribution_proven": False,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("arrivals", type=Path, help="decoded operations query result")
    parser.add_argument("observation", nargs="?", type=Path,
                        help="independent read JSON; omit to print the DFHack console command")
    args = parser.parse_args()
    try:
        arrivals = load(args.arrivals)
        if args.observation is None:
            print("dfmcp-furniture-materials", *(row["native_item_id"] for row in selected(arrivals)))
        else:
            print(json.dumps(compare(arrivals, load(args.observation)), indent=2, ensure_ascii=True))
    except (KeyError, TypeError, ValueError, OSError, AttributeError) as error:
        parser.exit(1, f"Furniture material comparison refused: {error}\n")


if __name__ == "__main__":
    main()
