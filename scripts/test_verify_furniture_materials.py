import copy
import hashlib
import unittest

from verify_furniture_materials import compare, selected


class FurnitureMaterials(unittest.TestCase):
    def setUp(self):
        folder = "region1"
        site = 3
        digest = hashlib.sha256(b"dfmcp-live-fortress-id-v1\0region1\0\x00\x00\x00\x03").digest()
        anchor = {"fortress_id": str(int.from_bytes(digest[:8], "big") | 1), "game_tick": 403202}
        rows = [{"native_item_id": 561 + i, "subtype": -1, "material": [419, i]} for i in range(3)]
        self.query = {
            "ok": True, "kind": "historical_furniture_arrivals", "anchor": anchor,
            "comparison": {"to": {"record_digest": "1" * 64, "anchor": anchor}},
            "new_item_count": 3, "before_next_item_id": 561, "after_next_item_id": 564,
            "target_source": {"df_version": "53.16", "dfhack_version": "53.16-r1.1", "paused": True},
            "new_item_examples": rows, "examples_truncated": False, "native_type_key": "BED",
        }
        items = [dict(row, type_key="BED", resolved=False) for row in rows]
        items[0].update(resolved=True, mode="plant", wood_flag=True, token="PLANT:PECAN:WOOD")
        items[1].update(resolved=True, mode="plant", wood_flag=False, token="PLANT:PECAN:NUT")
        self.obs = dict(self.query["target_source"], schema="dfmcp.furniture_material_observation/1",
                        world_folder=folder, site_id=site, year=1, year_tick=2, next_item_id=564, items=items)

    def test_classify_wood_separately_from_other_and_unknown(self):
        result = compare(self.query, self.obs)
        self.assertEqual([r["classification"] for r in result["items"]], ["plant_wood", "not_plant_wood", "unknown"])
        self.assertEqual((result["selected_plant_wood_count"], result["selected_unknown_material_count"]), (1, 1))
        self.assertFalse(result["order_attribution_proven"])
        self.obs["items"][0]["mode"] = "creature"
        self.assertEqual(compare(self.query, self.obs)["selected_plant_wood_count"], 0)

    def test_mismatched_sources_and_items_refuse_entire_result(self):
        for field, wrong in [("world_folder", "region2"), ("site_id", 4), ("year_tick", 3),
                             ("next_item_id", 565), ("dfhack_version", "different"), ("paused", False)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                compare(self.query, dict(self.obs, **{field: wrong}))
        for field, wrong in [("native_item_id", 561), ("material", [419, 6]), ("subtype", 1),
                             ("wood_flag", 1), ("token", "a\nb"), ("type_key", "TABLE")]:
            trial = copy.deepcopy(self.obs)
            trial["items"][1][field] = wrong
            with self.subTest(field=field), self.assertRaises(ValueError):
                compare(self.query, trial)

    def test_last_door_page_of_an_exhaustive_audit(self):
        self.query.update(native_type_key="DOOR", new_item_count=11, examples_skipped=8,
                          examples_from_item_id=561, examples_truncated=True, before_next_item_id=550)
        for item in self.obs["items"]:
            item["type_key"] = "DOOR"
        result = compare(self.query, self.obs)
        self.assertEqual((result["new_item_count"], result["examples_skipped"], len(result["items"])), (11, 8, 3))
        for wrong in [dict(examples_skipped=9), dict(examples_from_item_id=562), dict(examples_truncated=False)]:
            with self.subTest(wrong=wrong), self.assertRaises(ValueError):
                compare(dict(self.query, **wrong), self.obs)

    def test_only_returned_subset_may_be_classified(self):
        self.query.update(new_item_count=9, examples_truncated=True, after_next_item_id=570)
        for i in range(3, 8):
            row = {"native_item_id": 561+i, "subtype": -1, "material": [419, i]}
            self.query["new_item_examples"].append(row)
            self.obs["items"].append(dict(row, resolved=False, type_key="BED"))
        self.obs["next_item_id"] = 570
        result = compare(self.query, self.obs)
        self.assertTrue(result["examples_truncated"])
        self.assertEqual(len(result["items"]), 8)
        self.query["new_item_examples"].pop()
        with self.assertRaises(ValueError):
            selected(self.query)


if __name__ == "__main__":
    unittest.main()
