#!/usr/bin/env python3
"""Verify the pinned import and the documented, minimal upstream edits."""
from pathlib import Path
import hashlib
import json
import unittest
import importlib.util
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BW = ROOT / "crates/sim/bot_warfare"


class ImportContract(unittest.TestCase):
    def test_upstream_files_are_complete_and_unmodified_except_event_hook(self):
        manifest = json.loads((BW / "PROVENANCE.json").read_text())
        for name, expected in manifest["upstream_sha256"].items():
            data = (BW / name).read_bytes()
            if name in manifest.get("local_sha256",{}):
                expected=manifest["local_sha256"][name]
            elif name == "maps/mp/bots/_bot_utility.gsc":
                data = data.replace(b'    bw_event( self getentitynumber(), msg, a, b, c, d, e, f, g );\n', b'').replace(b'\n', b'\r\n')
            self.assertEqual(hashlib.sha256(data).hexdigest(), expected, name)
        for name,expected in manifest.get("local_sha256",{}).items():
            self.assertEqual(hashlib.sha256((BW/name).read_bytes()).hexdigest(),expected,name)
        source = (ROOT / "crates/sim/src/bot_warfare.rs").read_text()
        self.assertIn(manifest["revision"], source)
        workflow = (ROOT / ".github/workflows/ios-release.yml").read_text()
        self.assertNotIn("git apply", workflow)

    def test_analyzer_orders_rotating_segments_and_retains_errors(self):
        spec = importlib.util.spec_from_file_location("bw_report", ROOT / "tools/bw_report.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            (path / "manifest.json").write_text(json.dumps(dict(schema=1, controller="bot-warfare-gsc", source_revision="test", build="26", profile="summary", functions=[], natives=[])))
            (path / "trace-00.jsonl").write_text(json.dumps(dict(sequence=9,type="world",tick=40))+"\n{partial")
            (path / "trace-01.jsonl").write_text(json.dumps(dict(sequence=3,type="world",tick=20))+"\n")
            (path / "errors.json").write_text('[{"message":"retained outside ring"}]')
            result=mod.report(path)
            self.assertEqual(result["retained_sequence"],[3,9])
            self.assertEqual(result["malformed_lines"],1)
            self.assertEqual(result["latest_world"]["tick"],40)
            self.assertEqual(len(result["errors"]),1)

    def test_analyzer_sums_totals_but_keeps_the_largest_maximum(self):
        spec = importlib.util.spec_from_file_location("bw_report", ROOT / "tools/bw_report.py")
        mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            (path / "manifest.json").write_text(json.dumps(dict(schema=1, controller="bot-warfare-gsc", source_revision="test", build="26", profile="summary", functions=["function"], natives=["trace"])))
            records=[]
            for sequence, maximum in [(1,10),(2,20)]:
                row=dict(sequence=sequence,type="window",tick=sequence*100,frames=100,p50_ns=1,p95_ns=2,p99_ns=3,max_ns=maximum,gc_ns=0,diagnostic_write_ns=0,diagnostic_report_ns=0,dropped_records=0,io_errors=0)
                row["functions"]=[dict(id=0,instructions=100,segments=2,self_ns=200,max_slice_ns=maximum)]
                row["natives"]=[dict(id=0,calls=3,ns=100,max_ns=maximum,errors=0)]
                row["performance"] = dict(settle_runs=sequence, visibility_cached=sequence*2)
                records.append(row)
            (path / "trace-00.jsonl").write_text("\n".join(map(json.dumps,records))+"\n")
            result=mod.report(path)
            self.assertEqual(result["hot_functions"][0]["max_slice_ns"],20)
            self.assertEqual(result["hot_functions"][0]["instructions"],200)
            self.assertEqual(result["hot_natives"][0]["max_ns"],20)
            self.assertEqual(result["hot_natives"][0]["calls"],6)
            self.assertEqual(result["retained_ticks"],[100,200])
            self.assertEqual(result["performance_totals"],dict(settle_runs=3,visibility_cached=6))
            self.assertEqual(result["window_performance"][1]["visibility_cached"],4)


if __name__ == "__main__":
    unittest.main()
