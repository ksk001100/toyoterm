#!/usr/bin/env python3
"""Fixture/protocol and measurement-contract tests; no GUI or timing limits."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("baseline", Path(__file__).with_name("performance-baseline.py"))
BASELINE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASELINE)


class BaselineTests(unittest.TestCase):
    def test_bulk_sizes_and_windows_safe_writes(self):
        for content in ("ascii", "ansi", "unicode", "mixed"):
            data, repeat = BASELINE.payload({"scenario": "bulk", "content": content, "bytes": 1048576})
            self.assertEqual(len(data), 1048576)
            self.assertIsNone(repeat)
            chunks = list(BASELINE.utf8_chunks(data))
            self.assertEqual(b"".join(chunks), data)
            for chunk in chunks:
                chunk.decode("utf-8")  # Windows console writes must not split UTF-8.

    def test_redraw_transaction_and_grid(self):
        data, _ = BASELINE.payload({"scenario": "redraw", "columns": 200, "rows": 60, "redraws": 3})
        self.assertEqual(data.count(b"\x1b[?2026h"), 3)
        self.assertEqual(data.count(b"\x1b[?2026l"), 3)
        self.assertEqual(data.count(b"\x1b[60;1H"), 3)
        self.assertIn(b"\x1b[2K", data)

    def test_images_use_distinct_repeat_paths(self):
        for protocol in ("sixel", "kitty", "iterm2"):
            first, repeat = BASELINE.image_fixture(protocol, 300, 300)
            self.assertGreater(len(first), 0)
            if protocol == "kitty":
                self.assertIn(b"a=T,f=100", first)
                self.assertIn(b"a=p,i=7", repeat)
                self.assertNotIn(b"f=100", repeat)
            else:
                self.assertTrue(repeat.endswith(first))

    def test_incremental_log_reader_waits_for_complete_records(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "partial.log"
            path.write_text('TRACE toyoterm::perf: stage stage="frame" duration_us=1 ts_ns=', encoding="utf-8")
            self.assertEqual(BASELINE.events(path), [])
            with path.open("a", encoding="utf-8") as output:
                output.write('42\n')
            self.assertEqual(len(BASELINE.events(path)), 1)
            self.assertEqual(len(BASELINE.events(path)), 1)
            self.assertEqual(BASELINE.events(path)[0]["ts_ns"], 42)

    def test_percentiles_and_successful_present_count(self):
        self.assertGreaterEqual(BASELINE.stats([1, 9])["p95_ms"], BASELINE.stats([1, 9])["p50_ms"])
        rows = [{"stage": "frame", "duration_us": 9000, "ts_ns": 10},
                {"stage": "frame", "duration_us": 1000, "ts_ns": 20},
                {"stage": "present", "duration_us": 500, "ts_ns": 19},
                {"stage": "pty_event", "duration_us": 11000, "ts_ns": 12},
                {"stage": "present", "duration_us": 1, "ts_ns": 30}]
        result = BASELINE.summarize(rows, 10, 20)
        self.assertEqual(result["frame_count"], 1)
        self.assertEqual(result["main_thread_work_max_ms"], 11)
        self.assertEqual(result["pty_event_count"], 1)

    def test_parse_log_retains_nanosecond_integer(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "gui.log"
            path.write_text('TRACE toyoterm::perf: presented marker="done-p0-0" pane=4 ts_ns=1790959101835429900\n'
                            'WARN toyoterm::pty: ignored\n', encoding="utf-8")
            self.assertEqual(BASELINE.events(path), [{"marker": "done-p0-0", "pane": 4,
                                                      "ts_ns": 1790959101835429900}])


if __name__ == "__main__":
    unittest.main()
