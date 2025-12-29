# DEX Pool Monitor

업비트 KRW 상장 코인의 모든 DEX 풀 데이터를 실시간 수집하여 아비트라지 기회를 탐지하는 Rust 기반 고성능 도구.

## Features

- **다중 데이터 소스**: GeckoTerminal, 1inch, 0x, ParaSwap
- **실시간 CEX 가격**: Upbit WebSocket 연동
- **아비트라지 탐지**: DEX-DEX, DEX-CEX 가격 차이 감지
- **동적 LP 필터**: 스캠/허니팟 풀 자동 제외
- **최적화**: Rust 기반 저메모리, 고성능

## Quick Start

```bash
# 빌드
cargo build --release

# 실행
cargo run

# 서버 확인
curl http://localhost:3000/health
```

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| GET /health | 서버 상태 |
| GET /pools | 모든 풀 수집 |
| GET /pools/cached | 캐시된 풀 |
| GET /arbitrage | 아비트라지 기회 |
| WS /ws | 실시간 업데이트 |

## Configuration

`config.toml`:
```toml
[arbitrage]
threshold = 0.01  # 1% 가격 차이 시 알림

[filter]
min_lp = 1000     # 최소 LP (USD)
min_volume = 100  # 최소 거래량

[server]
port = 3000
```

## Tech Stack

- **Language**: Rust
- **Framework**: Axum
- **Async Runtime**: Tokio
- **HTTP Client**: Reqwest

## License

MIT
