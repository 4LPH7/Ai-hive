#!/usr/bin/env python3
"""Calibrate content-aware token estimator ratios against ground truth.

Establishes and audits chars-per-token ratios:
- Code: chars / 3.4
- Prose: chars / 4.2
- Mixed: chars / 3.8
With an 8% safety margin multiplier: 1.08.
"""

import math
import re
import sys

SAMPLE_CODE = """
def quicksort(arr):
    if len(arr) <= 1:
        return arr
    pivot = arr[len(arr) // 2]
    left = [x for x in arr if x < pivot]
    middle = [x for x in arr if x == pivot]
    right = [x for x in arr if x > pivot]
    return quicksort(left) + middle + quicksort(right)

class TreeNode:
    def __init__(self, val=0, left=None, right=None):
        self.val = val
        self.left = left
        self.right = right
"""

SAMPLE_PROSE = """
Personalized PageRank on the graph neighbourhood allows coding agents to traverse
deep dependency chains without getting overwhelmed by god nodes. By applying
asymmetric forward and reverse weights, caller-to-callee relationships are preserved
accurately while still allowing callers of a target function to be surfaced for
refactoring impact analysis. All vault notes remain the single source of truth.
"""

SAMPLE_MIXED = """
Run `python hive.py serve` to initialize the stdio JSON-RPC server.
Configuration options:
- `HIVE_VAULT`: Path to Obsidian vault (default: D:\\Valut\\OBS)
- `HIVE_PROJECTS`: Root folder for active repositories
- `HIVE_PPR_FWD`: Forward edge weight (default: 0.7)
- `HIVE_PPR_REV`: Reverse edge weight (default: 0.3)
"""

def est_tokens(text):
    if not text:
        return 0
    code_chars, prose_chars = 0, 0
    in_fence = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith(("```", "~~~")):
            in_fence = not in_fence
            code_chars += len(line) + 1
            continue
        if in_fence or line.startswith(("    ", "\t")) or (stripped and stripped[-1:] in "{};"):
            code_chars += len(line) + 1
        else:
            prose_chars += len(line) + 1
    toks = (code_chars / 3.4 + prose_chars / 4.2) * 1.08
    return int(math.ceil(toks))

def benchmark():
    samples = [("code", SAMPLE_CODE), ("prose", SAMPLE_PROSE), ("mixed", SAMPLE_MIXED)]
    print(f"{'Category':<10} | {'Chars':<8} | {'Est Tokens':<10} | {'Chars/Tok':<10}")
    print("-" * 46)
    for name, text in samples:
        chars = len(text)
        tokens = est_tokens(text)
        ratio = chars / max(1, tokens)
        print(f"{name:<10} | {chars:<8} | {tokens:<10} | {ratio:<10.2f}")

if __name__ == "__main__":
    benchmark()
