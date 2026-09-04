import time
import random
import threading
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
import uvicorn

try:
    import secure_kv_plane
except ImportError:
    print("❌ secure_kv_plane not found. Run: maturin develop")
    exit(1)

app = FastAPI(title="Secure KV Plane - Rust Powered")

TENANT_SECRETS = {
    "tenant-a": b"super-secret-key-for-tenant-a",
    "tenant-b": b"different-secret-for-tenant-b",
    "attacker": b"attacker-secret",
}

CACHE_STORE = {}
CACHE_LOCK = threading.Lock()

def get_cache(tenant_id: str):
    if tenant_id not in TENANT_SECRETS:
        TENANT_SECRETS[tenant_id] = f"auto-{tenant_id}".encode()
    with CACHE_LOCK:
        if tenant_id not in CACHE_STORE:
            secret = TENANT_SECRETS[tenant_id]
            CACHE_STORE[tenant_id] = secure_kv_plane.PySecureCache(secret)
        return CACHE_STORE[tenant_id]

class SeedRequest(BaseModel):
    prompt: str
    tenant_id: str = "tenant-a"

class SeedResponse(BaseModel):
    status: str
    entries: int

class PredictRequest(BaseModel):
    prompt: str
    tenant_id: str = "tenant-a"

class PredictResponse(BaseModel):
    text: str
    cached: bool
    latency_ms: float

def tokenize_mock(prompt: str) -> list[int]:
    if prompt == "Patient X diagnosed with Cancer":
        return [1, 2, 3, 4]
    elif prompt.startswith("Patient X diagnosed with "):
        suffix = prompt.replace("Patient X diagnosed with ", "")
        if suffix == "Diabetes":
            return [1, 2, 3, 5]
        elif suffix == "Flu":
            return [1, 2, 3, 6]
        elif suffix == "Asthma":
            return [1, 2, 3, 7]
        else:
            return [1, 2, 3, hash(suffix) % 100 + 10]
    return [hash(char) % 50 for char in prompt[:10]]

@app.post("/admin/seed", response_model=SeedResponse)
async def seed_cache(request: SeedRequest):
    prompt = request.prompt.strip()
    tenant_id = request.tenant_id
    if not prompt:
        raise HTTPException(status_code=400, detail="Empty prompt")
    cache = get_cache(tenant_id)
    tokens = tokenize_mock(prompt)
    cache.insert(tenant_id, tokens, prompt.encode())
    return SeedResponse(status="seeded", entries=1)

@app.post("/v1/predict", response_model=PredictResponse)
async def predict(request: PredictRequest):
    prompt = request.prompt.strip()
    tenant_id = request.tenant_id
    if not prompt:
        raise HTTPException(status_code=400, detail="Empty prompt")
    cache = get_cache(tenant_id)
    tokens = tokenize_mock(prompt)
    matched_len, block_id = cache.match_prefix(tenant_id, tokens)
    if matched_len > 0 and block_id is not None:
        is_hit = True
        base = random.uniform(0.020, 0.050)
    else:
        is_hit = False
        base = random.uniform(0.100, 0.200)
    jitter = random.uniform(-0.005, 0.005)
    latency_s = max(0.010, base + jitter)
    time.sleep(latency_s)
    text = f"mock completion (cached prefix: {block_id[:8] if block_id else 'None'}...)" if is_hit else "mock completion (recomputed)"
    return PredictResponse(text=text, cached=is_hit, latency_ms=round(latency_s * 1000, 2))

@app.get("/admin/cache")
async def view_cache():
    return {"message": "Cache is in Rust; check logs."}

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=8000)
