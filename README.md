# 🔐 Secure KV Plane

**Tenant-Isolated, Timing-Safe KV Cache for Multi-Tenant LLM Inference**

[![Rust](https://img.shields.io/badge/Rust-1.80+-orange)](https://www.rust-lang.org/)
[![Python](https://img.shields.io/badge/Python-3.10+-blue)](https://www.python.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

---

## 🚨 The Problem: Timing Side-Channels

In multi-tenant LLM serving (like vLLM), **prefix caching** reduces latency and cost. However, it creates a **timing side-channel**:

> An attacker (Tenant B) can infer a victim's (Tenant A) private prompt by measuring **Time-To-First-Token (TTFT)** differences.
> - **Cache Hit:** Fast response (prefix already computed).
> - **Cache Miss:** Slow response (full prefill required).

**Our Week 1 POC** proved this is exploitable. With just 40 probes, an attacker identified a private token (`"Cancer"`) with a **92.92ms** latency separation.

---

## 🛡️ The Solution: Secure KV Plane

**Secure KV Plane** is a Rust-based cryptographic memory layer that eliminates this leak.

### Core Features
1.  **HMAC-SHA256 Salting:**
    *   Cache Key = `HMAC(tenant_secret, tenant_id || SHA256(tokens))`
    *   **Result:** Tenant A and Tenant B generate mathematically different keys for the same prompt. Cross-tenant hits are impossible.
2.  **Per-Tenant Radix Trees:**
    *   Isolated prefix matching structures prevent memory side-channels.
3.  **LRU Eviction:**
    *   Automatic cleanup of old blocks to prevent OOM (Out of Memory).
4.  **Thread-Safe:**
    *   Built with `parking_lot::RwLock` for high-concurrency inference workloads.

---

## 🧪 The Proof (Attack vs. Defense)

We simulated a timing attack on a mock inference server.

### ❌ Before (Vulnerable)
The attacker successfully identified the cached token due to high latency separation.
```text
Target Token: "Cancer"
Median TTFT (Hit):  32.77 ms
Median TTFT (Miss): 125.69 ms
Separation Score:   92.92 ms  <-- ATTACK SUCCESSFUL

✅ After (Secured)
With HMAC salting, the attacker gets no cache hits. All probes are misses. The latency difference is just system noise.

Target Token: "Cancer"
Median TTFT (All Probes): ~150 ms
Separation Score:   0.24 ms   <-- ATTACK BLOCKED

 Architecture

 ┌─────────────────────────────────────────────────────┐
│                   Python Server                     │
│                 (FastAPI + Rust)                    │
├─────────────────────────────────────────────────────┤
│                    PyO3 Bridge                      │
├─────────────────────────────────────────────────────┤
│                 Rust Core Library                   │
│  ┌───────────────────────────────────────────────┐  │
│  │          Tenant-Isolated Cache                │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐   │  │
│  │  │Tenant A  │  │Tenant B  │  │Tenant C  │   │  │
│  │  │ HMAC     │  │ HMAC     │  │ HMAC     │   │  │
│  │  │Radix Tree│  │Radix Tree│  │Radix Tree│   │  │
│  │  └──────────┘  └──────────┘  └──────────┘   │  │
│  └───────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘

Getting Started
Prerequisites
Rust: 1.80+
Python: 3.10+
Maturin: pip install maturin
Installation

git clone https://github.com/Hamid0004/secure-kv-plane.git
cd secure-kv-plane
python -m venv .venv
source .venv/bin/activate
pip install maturin rich httpx fastapi uvicorn
maturin develop

Usage
Run the mock server:
python src/server.py

Run the attack simulation (in a new terminal):
python src/attacker.py

Roadmap
Phase 1: Core Rust Logic (HMAC, Radix Tree, LRU).
Phase 2: PyO3 Bindings & Attack Simulation.
Phase 3: Integration with vLLM KVConnector.
Phase 4: Adaptive Timing Jitter (Noise Injection).

Built with ❤️ by Hamid | ⭐ Star this repo if you found it useful!