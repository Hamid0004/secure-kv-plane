# Engineering Specification & Threat Model: Secure KV Plane (Week 1)

**Document Version:** 1.0.0  
**Phase:** Week 1 — Simulation & Timing Baseline  
**Scope:** Mock server simulation of LLM Key-Value (KV) cache prefix timing side-channels.

---

## 1. Threat Model

### 1.1 Victim Scenario
- **Victim Prompt:** `"Patient X diagnosed with Cancer"`
- The victim sends this prompt to the shared LLM inference server.
- The server caches the KV cache for the entire prompt prefix (`"Patient X diagnosed with"`).
- The victim is a paying tenant on the same shared infrastructure.

### 1.2 Attacker Capabilities
- **Knows:** The public prefix `"Patient X diagnosed with"`.
- **Can send:** API requests to the same server (multi-tenant environment).
- **Can measure:** Time-to-first-token (TTFT) / Time-to-first-byte for each request.
- **Cannot:** Read server memory directly, access other tenants' logs, or modify server code.
- **Goal:** Infer the secret token (`Cancer`) by probing candidate suffixes.

### 1.3 Root Cause
- **Cache Hit:** If the prompt prefix matches the cached KV blocks, the server skips the expensive prefill phase and jumps directly to decoding. TTFT is **fast**.
- **Cache Miss:** If the prefix differs (even by one character/token), the server recomputes the KV cache for the entire prompt. TTFT is **slow**.
- **Observability:** The difference between hit and miss latency is large enough to be measured over the network, even with noise.

---

## 2. Experimental Setup (Mock Server)

To isolate the timing effect from model inference variability, we use a deterministic mock server.

### 2.1 Mock Server Behavior
- **Endpoint:** `POST /generate`
- **Cache Key:** First 25 characters of the `prompt` field (exact string match).
- **Cache Hit Condition:** If the first 25 characters match a cached prefix.
- **Hit Latency:** `sleep(0.02 to 0.05 seconds)` + `random jitter (±5ms)`.
- **Miss Latency:** `sleep(0.10 to 0.20 seconds)` + `random jitter (±5ms)`.
- **Admin Endpoint:** `POST /admin/seed` to preload the victim prefix `"Patient X diagnosed with"` into the cache.

### 2.2 Why These Numbers?
- **20-50ms Hit:** Simulates fast prefill bypass.
- **100-200ms Miss:** Simulates full prefill computation.
- **Jitter:** Simulates realistic network/CPU fluctuations.

---

## 3. Attack Execution Steps

### 3.1 Attacker Knowledge
- Base prefix: `"Patient X diagnosed with"`
- Candidate tokens: `["Cancer", "Diabetes", "Flu", "Asthma"]`
- The attacker knows one of these is the actual victim suffix.

### 3.2 Probing Procedure
1. For each candidate `suffix`, construct the full prompt:
   - `probe = base_prefix + " " + suffix`
2. Send `N = 10` requests per candidate to the mock server.
3. Measure the round-trip time (RTT) or TTFT using `time.perf_counter()`.
4. Record all latencies in a CSV.

### 3.3 Inference Rule
- Compute the **median** latency for each candidate.
- The candidate with the **lowest median latency** is inferred to be cached (i.e., the victim's secret token).

---

## 4. Metrics & Success Criteria

| Metric | Definition | Target Value |
| :--- | :--- | :--- |
| **Hit Median Latency** | Median TTFT for cached probes. | **~35 ms** |
| **Miss Median Latency** | Median TTFT for non-cached probes. | **~150 ms** |
| **Separation Score** | `Miss_Median - Hit_Median` | **> 50 ms** |
| **Attack Success** | "Cancer" ranks as the #1 lowest latency candidate. | **100%** (in controlled mock) |

**Week 1 Win Condition:**
- The attacker script outputs "Cancer" as the predicted cached token.
- The latency distribution plot shows clear separation between hit and miss groups.

---

## 5. Defenses (Introduction for Week 2)

This experiment validates the threat. Defenses to be built later:

1.  **Tenant Salting:** Cache key = `HMAC(tenant_secret, token_ids)` so Tenant A cannot hit Tenant B's cache.
2.  **Public/Private Split:** Separate cache pools for common system prompts vs. user-specific data.
3.  **Timing Padding:** Add random delays to normalize hit/miss latency.
4.  **Probe Detection:** Rate-limit identical prefixes to prevent brute-force guessing.

---

## 6. Ethical Boundaries

- All experiments run on **localhost** only.
- No real user data, no production APIs, no third-party services.
- This is a research simulation to build a defensive security layer.

---

## 7. Next Steps (Day 2)

1.  Build the mock server (`src/mock_server.py`) matching this spec.
2.  Write a health-check client to verify hit vs miss latency.
3.  Document server startup commands.

