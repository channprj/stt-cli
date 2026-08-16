# stt-cli

[English](README.md) | **한국어**

> 오디오를 전사하고, 각 줄에 그 말이 실제로 나온 시각을 찍어줍니다.

전사 모델은 "이 문장은 305초 지점에서 시작한다"처럼 **오프셋**만 알려줍니다.
`stt-cli`는 녹음기가 파일 이름에 남겨둔 시작 시각을 읽어서 그 오프셋을 더합니다.
그러면 몇 분 몇 초 지점인지가 아니라, 몇 시 몇 분에 한 말인지가 남습니다.

```console
$ stt-cli transcribe 20260815_143000_standup.m4a
→ recording starts 2026-08-15 14:30:00 (from the file name)
→ uploading to OpenAI (whisper-1)
[2026-08-15 14:30:05] 좋은 아침입니다. 어제 배포는 무사히 끝났습니다.
[2026-08-15 14:30:19] 오늘은 인덱싱 쪽을 보겠습니다.
```

녹음이 `14:30:00`에 시작했으니, 5초 지점의 발화는 `14:30:05`이 됩니다.

## 주요 기능

- 파일 이름의 날짜·시각을 읽어 절대 시각 타임스탬프 생성
- 파일 이름에 정보가 없으면 상대 오프셋으로 물러서고, 그 사실을 알려줌
- `--start`로 시작 시각 직접 지정
- OpenAI와 Soniox 두 가지 백엔드
- API 키를 `~/.config/stt-cli/api.json`에 `0600`으로 보관
- 색이 입혀진 도움말, 파이프로 넘길 때는 자동으로 색 제거

## 빠른 시작

### macOS (Homebrew)

```sh
brew install --HEAD channprj/tap/stt-cli
```

> **참고:** `--HEAD`가 반드시 필요합니다. `channprj/stt-cli`는 비공개 저장소라
> 익명으로 받을 수 있는 릴리즈 tarball이 없습니다. 그래서 포뮬러는 tarball 대신
> git clone으로 설치하며, 이때 이미 로그인된 git 자격 증명을 그대로 씁니다.
> 저장소 읽기 권한과 GitHub 로그인(`gh auth login` 정도면 충분)이 필요합니다.

업데이트할 때는 `--fetch-HEAD`를 붙여야 새 커밋을 확인합니다.

```sh
brew upgrade --fetch-HEAD stt-cli
```

### 소스에서 빌드

```sh
git clone https://github.com/channprj/stt-cli.git
cd stt-cli
cargo install --path .
```

Rust 1.85 이상이 필요합니다. 설치하지 않고 쓰려면 `cargo build --release`로
빌드한 뒤 `target/release/stt-cli`를 쓰면 됩니다.

## API 키 등록

둘 중 계정이 있는 쪽을 쓰면 됩니다.

```sh
stt-cli config set openai      # 프롬프트에 키를 붙여넣기
stt-cli config set soniox
stt-cli config default soniox  # --provider 생략 시 사용할 기본값
stt-cli config show
```

키는 `~/.config/stt-cli/api.json`에 권한 `0600`으로 저장되며,
`XDG_CONFIG_HOME`을 설정해두었다면 그쪽을 따릅니다. 키를 인자로 바로 넘길 수도
있지만, 프롬프트로 입력하면 셸 히스토리에 남지 않습니다.

`OPENAI_API_KEY`, `SONIOX_API_KEY` 환경 변수가 저장된 키보다 우선하므로,
한 번만 쓸 때는 설정 없이 그냥 실행해도 됩니다.

## 사용법

```sh
stt-cli transcribe recording.m4a                         # 타임스탬프가 찍힌 텍스트를 stdout으로
stt-cli transcribe recording.m4a -f json -o out.json     # 기계가 읽을 형식
stt-cli transcribe recording.m4a -l ko                   # 언어 힌트
stt-cli transcribe recording.m4a -p soniox               # 백엔드 선택
stt-cli transcribe rec.m4a --start "2026-08-15 14:30"    # 시작 시각 직접 지정
```

인자 없이 `stt-cli`만 치면 전체 도움말이 나옵니다. 폴더 단위 처리, 녹음 파일
이름 짓는 요령, JSON 후처리 같은 실제 작업 흐름은
[docs/USAGE.md](docs/USAGE.md)에 정리해두었습니다.

### 파일 이름에서 읽어내는 시작 시각

아래는 모두 `2026-08-15 14:30:22`로 읽힙니다.

| 파일 이름 |
|---|
| `20260815_143022.m4a` |
| `20260815143022.wav` |
| `2026-08-15_14-30-22.mp3` |
| `2026-08-15 14.30.22.m4a` |
| `2026-08-15T14:30:22.flac` |
| `New Recording 2026-08-15 at 14.30.22.m4a` |
| `IMG_20260815_143022.mov` |

초는 없어도 됩니다(`zoom_20260815_1430.mp4`). 시각 없이 날짜만 있으면 자정으로
잡습니다(`notes-2026-08-15.wav` → `2026-08-15 00:00:00`).

날짜로 볼 만한 것이 전혀 없으면 시각을 지어내지 않고, 그 사실을 알린 뒤
`[00:00:05]` 같은 상대 오프셋으로 물러섭니다. `--start`는 파일 이름과 같은
표기를 받으며 언제나 우선합니다.

시각은 시간대 변환 없이 로컬 벽시계 시각 그대로 다룹니다.

### 출력

기본값인 `-f text`는 발화 한 건당 한 줄입니다.

```
[2026-08-15 14:30:05] 좋은 아침입니다.
```

`-f json`은 오프셋과 절대 시각을 함께 남기므로, 나중에 다시 계산하거나
후처리하기 좋습니다.

```json
{
  "file": "20260815_143000_standup.m4a",
  "provider": "openai",
  "model": "whisper-1",
  "started_at": "2026-08-15T14:30:00",
  "segments": [
    { "start": 5.0, "end": 8.2, "at": "2026-08-15T14:30:05", "text": "좋은 아침입니다." }
  ]
}
```

시작 시각을 모를 때는 `started_at`과 `at`을 아예 넣지 않습니다.

## 제공자 비교

| | OpenAI | Soniox |
|---|---|---|
| 기본 모델 | `whisper-1` | `stt-async-v5` |
| 타임스탬프 | 세그먼트 단위 | 토큰 단위, 발화로 묶어서 출력 |
| 업로드 한도 | 25 MB | 사실상 없음 |
| 동작 방식 | 요청 한 번 | 업로드 → 폴링 → 조회 |

모델은 `-m`으로 바꿀 수 있습니다. OpenAI에서는 `whisper-*` 계열만 타이밍을
돌려줍니다. `gpt-4o-transcribe` 계열은 텍스트만 주기 때문에, 그 모델을 지정하면
경고를 띄우고 세그먼트 하나로 처리합니다.

Soniox에 올린 파일은 전사 결과를 받아온 뒤 Soniox에서 삭제합니다.

OpenAI에서 25 MB를 넘기면 제공자를 바꾸거나 파일을 나누세요.

```sh
ffmpeg -i long.m4a -f segment -segment_time 900 -c copy part%03d.m4a
```

나뉜 조각마다 자기 시작 시각이 담기도록 이름을 붙이면, 잘라낸 뒤에도
타임스탬프가 어긋나지 않습니다.

## 릴리즈

릴리즈 버전은 저장소 루트의 `VERSION` 파일에 `v{MAJOR}.{YYMMDD}.{PATCH}`
형식으로 두고 바이너리에 그대로 컴파일해 넣습니다. 그래서 `stt-cli --version`
출력과 태그가 어긋날 수 없습니다. 버전을 올리고 같은 이름의 태그를 밀거나,
Release 워크플로를 수동으로 실행하면 됩니다.

```sh
printf 'v1.260816.0\n' > VERSION
git commit -am "chore: release v1.260816.0"
git tag v1.260816.0 && git push origin main --tags
```

Homebrew 포뮬러는 tarball이 아니라 `main`을 따라가므로, 릴리즈할 때 tap을
건드릴 필요가 없습니다.

## 개발

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
