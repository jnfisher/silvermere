"""The M3 scenario group (scenarios/m3): every file loads, its setup scripts exist, and the
two-connection scripts open each connection before they use it (replay.py `as`)."""
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import differential  # noqa: E402

M3 = os.path.join(differential.SCENARIOS, "m3")


def read(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


class M3Scenarios(unittest.TestCase):
    def test_group_lists_the_scenarios(self):
        self.assertEqual(differential.group_scenarios("m3"),
                         ["m3/bank", "m3/comms", "m3/party", "m3/shops"])

    def test_each_scenario_loads_with_existing_setup_and_script(self):
        for name in differential.group_scenarios("m3"):
            sc = differential.load_scenario(name)
            self.assertTrue(os.path.isfile(sc["script"]), name)
            for entry in sc["setup"]:
                path = (os.path.join(differential.SCENARIOS, entry["script"]) if entry.get("script")
                        else os.path.join(differential.CAPTURES, entry["scenario"], "script.json"))
                self.assertTrue(os.path.isfile(path), f"{name}: {entry}")

    def test_bank_is_not_rerun_after_a_wanderer_and_its_exit_is_not_diffed(self):
        """main's rerun after interference (T-057) plays m3/bank once more after a wanderer's
        arrival, but Isolde is then in the Bank of Godfrey, not at the start (the c17ec7e run:
        step 5 `se` found no exit and both sides timed out). The scenario opts out
        (`no_rerun`); the exit step, where the arrival came, is `random_steps`; the bank
        commands before it are still compared."""
        sc = differential.load_scenario("m3/bank")
        script = read(sc["script"])
        sends = [st.get("send") for st in script["steps"]]
        x = max(i for i, t in enumerate(sends) if t == "x")
        self.assertTrue(sc["no_rerun"])
        self.assertNotIn("rerun_restore", sc)
        self.assertEqual(sc["random_steps"], [x])
        self.assertIn(sends.index("list"), range(max(sc["ignore_steps"]) + 1, x))
        arrival = [{"step": x, "name": "monster_arrives_generic", "fields": {}}]
        self.assertEqual(differential.interference(sc, [], arrival, []), [])
        before = [{"step": sends.index("deposit 5"), "name": "monster_arrives_generic", "fields": {}}]
        self.assertEqual(len(differential.interference(sc, [], before, [])), 1)

    def test_connections_open_before_use_and_waits_search_from_the_last_send(self):
        for name in differential.group_scenarios("m3"):
            script = read(differential.load_scenario(name)["script"])
            open_ = set()
            for i, st in enumerate(script["steps"]):
                who = st.get("as", "A")
                if st.get("op") == "connect":
                    open_.add(who)
                self.assertIn(who, open_, f"{name} step {i}")
                if st.get("op") == "sleep" and st.get("until"):
                    self.assertEqual(st.get("since"), "last", f"{name} step {i}")
                if "send" in st:
                    self.assertTrue(st.get("until") or st.get("nowait") or st["send"] == "Y",
                                    f"{name} step {i}: {st['send']!r} waits for nothing")


if __name__ == "__main__":
    unittest.main()
