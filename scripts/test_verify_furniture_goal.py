import copy
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from verify_furniture_goal import evaluate


def fixture(count=19):
    fortress = str(int.from_bytes(hashlib.sha256(b"dfmcp-live-fortress-id-v1\0site\0\0\0\0\1").digest()[:8],
                                  "big") | 1)
    first = {"record": 1, "record_digest": "1" * 64, "source_digest": "a" * 64,
             "anchor": {"fortress_id": fortress, "epoch": 0, "sequence": 0, "game_tick": 1, "state_hash": "e" * 64}}
    last = {"record": 2, "record_digest": "2" * 64, "source_digest": "b" * 64,
            "anchor": {"fortress_id": fortress, "epoch": 0, "sequence": 1, "game_tick": 2, "state_hash": "f" * 64}}
    pages = []
    for start in range(0, max(1, count), 8):
        examples = [{"native_item_id": 100 + 3*i, "material": [419, i%3], "subtype": -1}
                    for i in range(start, min(count, start+8))]
        request = {"schema": "dfmcp.query/1", "query": dict(kind="historical_furniture_arrivals",
                   furniture_kind="door", material=None, examples_from_item_id=100+3*(start+8))}
        for key, entry in (("from", first), ("to", last)):
            request["query"][key] = {k: entry[k] for k in ("record", "record_digest")}
        source = {"df_version": "df", "dfhack_version": "sdk", "paused": True}
        page = {"ok": True, "kind": "historical_furniture_arrivals", "native_type_key": "DOOR",
                "before_next_item_id": 95, "after_next_item_id": 100+3*count,
                "examples_from_item_id": 100+3*start if start else 95, "examples_skipped": start,
                "new_item_count": count, "examples_truncated": len(examples) != count,
                "before_item_count": 1, "after_item_count": count, "net_item_count_change": count-1,
                "material_filter": None, "new_item_examples": examples, "target_source": source,
                "next_query": request if start+8 < count else None,
                "comparison": {"journal_id": "4"*64, "profile": "operations/1.4",
                               "from": copy.deepcopy(first), "to": copy.deepcopy(last)},
                "anchor": copy.deepcopy(last["anchor"]), "basis": copy.deepcopy(first["anchor"])}
        items = []
        for example in examples:
            row = dict(example, type_key="DOOR", resolved=False)
            mat = row["material"][1]
            if mat in (0, 1):
                row.update(resolved=True, mode="plant", wood_flag=mat == 0,
                           token="PLANT:PECAN:" + ("WOOD" if mat == 0 else "NUT"))
            items.append(row)
        obs = dict(source, schema="dfmcp.furniture_material_observation/1", items=items, world_folder="site",
                   site_id=1, next_item_id=page["after_next_item_id"], year=0, year_tick=2)
        pages.append((page, obs if count else None))
    return pages


class Goal(unittest.TestCase):
    def test_complete_and_partial_three_way_outcomes(self):
        pages = fixture()
        for minimum, want in [(1, "met"), (7, "met"), (8, "unknown"), (13, "unknown"), (14, "insufficient")]:
            with self.subTest(minimum=minimum):
                r = evaluate(minimum, pages)
                self.assertEqual(r["outcome"], want)
                self.assertEqual((r["verified_plant_wood"], r["verified_other_material"],
                                  r["unclassified_selected"], r["unseen_arrivals"]), (7, 6, 6, 0))
                self.assertTrue(r["complete_arrival_coverage"])
                self.assertIsNone(r["next_query"])
                self.assertFalse(r["order_attribution_proven"])
        self.assertEqual(evaluate(3, pages[:1])["outcome"], "met")
        partial = evaluate(50, pages[:1])
        self.assertEqual(partial["outcome"], "unknown")
        self.assertEqual(partial["unseen_arrivals"], 11)
        self.assertIsNotNone(partial["next_query"])

    def test_missing_material_and_empty_inventory(self):
        pages = fixture()
        pages[1] = pages[1][0], None
        result = evaluate(7, pages)
        self.assertEqual(result["outcome"], "unknown")
        self.assertTrue(result["complete_arrival_coverage"])
        self.assertEqual(result["independent_reads"], 2)
        self.assertEqual(evaluate(20, pages)["outcome"], "insufficient")
        self.assertEqual(evaluate(1, fixture(0))["outcome"], "insufficient")

    def test_gaps_repeats_and_substitutions_refuse(self):
        pages = fixture()
        for subset in [[], pages[1:], [pages[0], pages[2]], [pages[0], pages[0]], pages[::-1], pages+[pages[-1]]]:
            with self.assertRaises(ValueError):
                evaluate(1, subset)
        for target, key, value in [("comparison", "journal_id", "5"*64), ("comparison", "profile", "operations/1.3"),
                                   ("to", "record_digest", "3"*64), ("from", "source_digest", "d"*64),
                                   ("page", "native_type_key", "BED"), ("page", "new_item_count", 21),
                                   ("page", "material_filter", [419, 0]), ("page", "examples_from_item_id", 123),
                                   ("page", "next_query", None)]:
            amended = copy.deepcopy(pages)
            page = amended[1][0]
            obj = {"page": page, "comparison": page["comparison"], "to": page["comparison"]["to"],
                   "from": page["comparison"]["from"]}[target]
            obj[key] = value
            with self.subTest(target=target, key=key), self.assertRaises(ValueError):
                evaluate(1, amended)
        pages[0][0]["next_query"]["query"]["examples_from_item_id"] += 1
        with self.assertRaises(ValueError):
            evaluate(1, pages)

    def test_independent_evidence_cannot_cross_pages_or_disagree(self):
        for corrupt in ("swap", "conflict"):
            pages = fixture()
            if corrupt == "swap":
                pages[1] = pages[1][0], pages[0][1]
            else:
                pages[1][1]["items"][1]["token"] = "PLANT:OAK:WOOD"
            with self.assertRaises(ValueError):
                evaluate(1, pages)

    def test_cli_with_multiple_pages(self):
        script = Path(__file__).with_name("verify_furniture_goal.py")
        with tempfile.TemporaryDirectory() as td:
            command = [sys.executable, str(script), "--minimum", "3"]
            for index, (page, obs) in enumerate(fixture(9)):
                p, o = Path(td) / f"p{index}", Path(td) / f"o{index}"
                p.write_text(json.dumps(page))
                o.write_text(json.dumps(obs))
                command.extend(["--page", str(p), str(o)])
            result = subprocess.run(command, capture_output=True, check=True, text=True)
            self.assertEqual(json.loads(result.stdout)["outcome"], "met")


if __name__ == "__main__":
    unittest.main()
