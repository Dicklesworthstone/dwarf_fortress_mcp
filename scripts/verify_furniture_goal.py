#!/usr/bin/env python3
"""Evaluate a minimum of new plant-wood furniture against one paged inventory comparison."""

import argparse
import json
import re
from pathlib import Path

from verify_furniture_materials import compare, integer, require, selected

PAGE_LIMIT = 8192
INPUT_LIMIT = 256 * 1024 * 1024


def digest(value):
    return isinstance(value, str) and re.fullmatch("[0-9a-f]{64}", value) is not None


def source_key(page):
    comparison = page["comparison"]
    first, last = comparison["from"], comparison["to"]
    require(digest(comparison["journal_id"]) and comparison["profile"] in ("operations/1.3", "operations/1.4"),
            "invalid comparison journal or profile")
    for entry, anchor in ((first, page["basis"]), (last, page["anchor"])):
        require(digest(entry["record_digest"]) and digest(entry["source_digest"])
                and integer(entry["record"], 1, 4096) and entry["anchor"] == anchor,
                "invalid comparison endpoint")
        require(digest(anchor["state_hash"]) and integer(anchor["game_tick"], 0, 2**64-1)
                and integer(anchor["epoch"], 0, 2**64-1) and integer(anchor["sequence"], 0, 2**64-1)
                and isinstance(anchor["fortress_id"], str) and re.fullmatch("[0-9]+", anchor["fortress_id"]),
                "invalid comparison anchor")
    require(first["record"] <= last["record"] and page["basis"]["fortress_id"] == page["anchor"]["fortress_id"]
            and page["basis"]["epoch"] == page["anchor"]["epoch"]
            and page["basis"]["game_tick"] <= page["anchor"]["game_tick"], "comparison endpoints differ in lineage or order")
    require(first["record"] != last["record"] or first == last, "same record has inconsistent identities")
    before, after, count = page["before_item_count"], page["after_item_count"], page["new_item_count"]
    require(integer(before, 0, 65536) and integer(after, 0, 65536)
            and count <= after and after - before == page["net_item_count_change"], "inconsistent stock counts")
    require(first["record"] != last["record"] or
            (count == 0 and before == after and page["before_next_item_id"] == page["after_next_item_id"]),
            "a single retained endpoint cannot show arrivals")
    source = page["target_source"]
    require(all(isinstance(source.get(key), str) and 0 < len(source[key].encode()) <= 128
                for key in ("df_version", "dfhack_version")), "invalid target software identity")
    selector = page["material_filter"]
    require(selector is None or (isinstance(selector, list) and len(selector) == 2
            and integer(selector[0], 0, 2**31-1) and integer(selector[1], -1, 2**31-1)), "invalid material filter")
    # The journal may append after these records. The selected records, their
    # source digests, anchors, filters and replay totals must remain identical.
    return (comparison["journal_id"], comparison["profile"], first, last, page["native_type_key"], selector,
            page["target_source"], page["before_next_item_id"], page["after_next_item_id"], before, after, count)


def continuation(page, rows, processed):
    request = page["next_query"]
    if processed == page["new_item_count"]:
        require(request is None, "unexpected continuation after the complete roster")
        return None
    require(rows and isinstance(request, dict) and request.get("schema") == "dfmcp.query/1",
            "incomplete page lacks a continuation")
    query = request["query"]
    floor = query["examples_from_item_id"]
    require(integer(floor, rows[-1]["native_item_id"] + 1, page["after_next_item_id"] - 1),
            "continuation does not advance within the native item frontier")
    for key in ("from", "to"):
        ref = page["comparison"][key]
        require(query[key] == {"record": ref["record"], "record_digest": ref["record_digest"]},
                "continuation substitutes an endpoint")
    require(query["kind"] == "historical_furniture_arrivals" and query["furniture_kind"] == page["native_type_key"].lower()
            and query["material"] == page["material_filter"], "continuation substitutes a selection")
    return floor


def evaluate(minimum, pairs):
    require(integer(minimum, 1, 65536), "minimum must be in 1..65536")
    identity = None
    known_materials = {}
    pages = observed_pages = processed = wood = other = unknown = 0
    next_floor = None
    for page, observation in pairs:
        require(pages < PAGE_LIMIT, "too many arrival pages")
        rows = selected(page, allow_empty=True)
        key = source_key(page)
        floor = page["examples_from_item_id"]
        require(integer(floor, page["before_next_item_id"], 2**31-1)
                and integer(page["examples_skipped"], 0, 65536) and page["examples_skipped"] == processed,
                "arrival pages have a gap or overlap")
        if identity is None:
            identity = key
            require(floor == page["before_next_item_id"], "audit must start at the original allocator frontier")
        else:
            require(key == identity, "pages belong to different comparisons")
            require(next_floor is not None and floor == next_floor and rows
                    and rows[0]["native_item_id"] == next_floor, "page differs from the preceding continuation")
        for row in rows:
            pair = row["material"]
            require(isinstance(pair, list) and len(pair) == 2 and all(integer(v, -1, 2**31-1) for v in pair)
                    and integer(row["subtype"], -1, 2**31-1)
                    and (page["material_filter"] is None or pair == page["material_filter"]),
                    "example is inconsistent with the selected inventory")
        pages += 1
        processed += len(rows)
        next_floor = continuation(page, rows, processed)
        if observation is None:
            unknown += len(rows)
            continue
        require(rows, "empty inventory page cannot have a selected-item observation")
        result = compare(page, observation)
        observed_pages += 1
        for row in result["items"]:
            if row["classification"] == "unknown":
                unknown += 1
                continue
            pair = tuple(row["material"])
            meaning = (row["mode"], row["wood_flag"], row["token"])
            require(known_materials.setdefault(pair, meaning) == meaning,
                    "independent reads disagree on the same material raw")
            if row["classification"] == "plant_wood":
                wood += 1
            else:
                other += 1
    require(pages, "at least one arrival page is required")
    complete = next_floor is None
    unseen = page["new_item_count"] - processed
    if wood >= minimum:
        outcome = "met"
    elif complete and wood + unknown < minimum:
        outcome = "insufficient"
    else:
        outcome = "unknown"
    return {
        "schema": "dfmcp.new_plant_wood_inventory_goal/1", "outcome": outcome, "minimum": minimum,
        "journal_id": identity[0], "profile": identity[1], "from": identity[2], "to": identity[3],
        "native_type_key": page["native_type_key"], "material_filter": page["material_filter"],
        "arrival_count": page["new_item_count"], "pages": pages, "independent_reads": observed_pages,
        "complete_arrival_coverage": complete, "verified_plant_wood": wood, "verified_other_material": other,
        "unclassified_selected": unknown, "unseen_arrivals": unseen, "next_query": page["next_query"],
        "same_process_incarnation_proven": False, "order_attribution_proven": False,
    }


def page_files(paths):
    remaining = INPUT_LIMIT

    def read(path):
        nonlocal remaining
        allowed = min(remaining, 2 * 1024 * 1024)
        with Path(path).open("rb") as stream:
            data = stream.read(allowed + 1)
        require(len(data) <= allowed, "input exceeds per-file or complete-audit byte allowance")
        remaining -= len(data)
        return json.loads(data)

    require(len(paths) <= PAGE_LIMIT, "too many arrival pages")
    for query, material in paths:
        yield read(query), None if material == "-" else read(material)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--minimum", required=True, type=int)
    parser.add_argument("--page", nargs=2, required=True, action="append", metavar=("ARRIVALS", "MATERIALS"),
                        help="one archive page and its independent observation, or - for missing material evidence")
    args = parser.parse_args()
    try:
        print(json.dumps(evaluate(args.minimum, page_files(args.page)), indent=2))
    except (KeyError, TypeError, ValueError, OSError, AttributeError) as error:
        parser.exit(1, f"Inventory goal assessment refused: {error}\n")
