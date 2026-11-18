#!/usr/bin/env python3
"""Summarize a BotWarfareDiagnostics session without third-party dependencies."""
import argparse
import collections
import json
from pathlib import Path


def report(folder):
    manifest = json.loads((folder / "manifest.json").read_text())
    records, malformed = {}, 0
    for path in folder.glob("trace-*.jsonl"):
        for line in path.read_text(errors="replace").splitlines():
            try:
                record = json.loads(line)
                records[record["sequence"]] = record
            except (ValueError, KeyError):
                malformed += 1
    ordered = [records[k] for k in sorted(records)]
    functions, natives = {}, {}
    windows = [r for r in ordered if r["type"] == "window"]
    performance = collections.Counter()
    for record in windows:
        performance.update(record.get("performance", {}))
        for kind, totals in (("functions", functions), ("natives", natives)):
            for row in record[kind]:
                acc = totals.setdefault(row["id"], collections.Counter())
                for key, value in row.items():
                    if key.startswith("max_"):
                        acc[key] = max(acc[key], value)
                    elif key != "id":
                        acc[key] += value
    def hottest(totals, names, timing):
        return [dict(name=names[i], **dict(counts)) for i, counts in
                sorted(totals.items(), key=lambda item: item[1][timing], reverse=True)[:20]]
    ticks = [r["tick"] for r in ordered if isinstance(r.get("tick"), int)]
    errors = json.loads((folder / "errors.json").read_text()) if (folder / "errors.json").exists() else []
    worlds = [r for r in ordered if r["type"] == "world"]
    return dict(schema=manifest["schema"], controller=manifest["controller"],
                source_revision=manifest["source_revision"], build=manifest["build"],
                profile=manifest["profile"], retained_records=len(ordered), malformed_lines=malformed,
                retained_sequence=[min(records), max(records)] if records else [],
                retained_ticks=[min(ticks), max(ticks)] if ticks else [],
                completed_windows=len(windows),
                window_percentiles=[{k:r[k] for k in ("tick","frames","p50_ns","p95_ns","p99_ns","max_ns","gc_ns","diagnostic_write_ns","diagnostic_report_ns","dropped_records","io_errors")} for r in windows],
                performance_totals=dict(performance),
                window_performance=[dict(tick=r["tick"], frames=r["frames"], **r.get("performance", {})) for r in windows],
                latest_world=worlds[-1] if worlds else None,
                errors=errors,
                hot_functions=hottest(functions,manifest["functions"],"self_ns"),
                hot_natives=hottest(natives,manifest["natives"],"ns"),
                caution="Retained rotating trace only; percentiles remain per window. Native timings include callbacks; do not add them to exclusive script times as an exact total. Detail capture changes performance. References and truncated snapshots are not a complete deterministic replay.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    result = json.dumps(report(args.session), indent=2) + "\n"
    if args.out:
        args.out.write_text(result)
    else:
        print(result, end="")
