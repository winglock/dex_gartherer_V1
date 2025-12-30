import json
import os
from pathlib import Path

data_dir = Path("upbit_matcha_tokens_2025-12-30T01-58-55")
output = {}

for folder in data_dir.iterdir():
    if not folder.is_dir():
        continue
    
    symbol = folder.name
    data_file = folder / f"{symbol}_data.json"
    
    if not data_file.exists():
        continue
    
    try:
        with open(data_file, "r", encoding="utf-8") as f:
            data = json.load(f)
        
        if not data.get("success") or not data.get("tokens"):
            continue
        
        # Find best token: exact match, isInDex=true, not honeypot, highest volume
        best_tokens = []
        for token in data["tokens"]:
            token_symbol = token.get("symbol", "").upper()
            
            # Exact match or case-insensitive match
            if token_symbol == symbol.upper():
                is_in_dex = token.get("isInDex", False)
                is_honeypot = token.get("isHoneypot", False)
                volume = token.get("volumeUsdMonthly", 0) or 0
                
                if is_in_dex and not is_honeypot:
                    best_tokens.append({
                        "address": token["address"],
                        "chainId": token["chainId"],
                        "decimals": token.get("decimals", 18),
                        "volume": volume
                    })
        
        # Sort by volume, take top ones per chain
        best_tokens.sort(key=lambda x: x["volume"], reverse=True)
        
        chains = {}
        for t in best_tokens:
            chain_id = str(t["chainId"])
            if chain_id not in chains:
                chains[chain_id] = t["address"]
        
        if chains:
            output[symbol] = chains
            print(f"✓ {symbol}: {len(chains)} chains")
    
    except Exception as e:
        print(f"✗ {symbol}: {e}")

# Save consolidated JSON
with open("matcha_tokens_consolidated.json", "w", encoding="utf-8") as f:
    json.dump(output, f, indent=2)

print(f"\n=== Total: {len(output)} tokens ===")
