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

- 흔히 쓰는 녹음 파일 이름에서 로컬 벽시계 시작 시각을 읽거나 명시적인
  `--start` 값을 받습니다.
- OpenAI, Soniox, Groq를 지원하며 실행할 때마다 제공자와 모델을 바꿀 수
  있습니다.
- `text`, `json`, `srt`, `vtt`, `txt`, `csv` 형식으로 출력합니다.
- `--dry-run`으로 실제 요청 정보와 예상 비용을 미리 확인합니다.
- `--vad`로 무음을 선택적으로 제거하면서 결과는 원본 녹음 타임라인에 다시
  맞춥니다.
- API 키를 접근 권한이 제한된 설정 파일에 저장하고 환경 변수로 덮어쓸 수
  있습니다.
- 릴리즈 설치본을 위한 GitHub Release 기반 업데이트 명령을 제공합니다.

## 설치

### Homebrew

```sh
brew install --HEAD channprj/tap/stt-cli
```

이 비공개 저장소에서는 `--HEAD`가 확실한 설치 경로입니다. 기존 GitHub 자격
증명을 사용해 최신 `main` 브랜치를 빌드합니다. 저장소 읽기 권한과 인증된 Git
설정이 필요하며, 일반적인 HTTPS 설정에서는 `gh auth login`이면 충분합니다.

head 설치본은 재설치해서 갱신합니다.

```sh
brew reinstall channprj/tap/stt-cli
```

### 소스에서 설치

```sh
git clone https://github.com/channprj/stt-cli.git
cd stt-cli
cargo install --path .
```

소스 빌드에는 Rust 1.85 이상이 필요합니다. 설치하지 않으려면
`cargo build --release`로 만든 `target/release/stt-cli`를 사용하면 됩니다.

## 빠른 시작

제공자 키 하나를 등록한 뒤 파일을 전사합니다.

```sh
stt-cli config set openai
stt-cli config show
stt-cli transcribe 20260815_143000_standup.m4a
```

대신 `soniox`나 `groq`를 등록해도 됩니다. `OPENAI_API_KEY`,
`SONIOX_API_KEY`, `GROQ_API_KEY`는 일회성 실행이나 자동화에서 저장된 키보다
우선합니다.

```sh
stt-cli transcribe meeting.m4a --start "2026-08-15 14:30" -l ko
stt-cli transcribe meeting.m4a -p groq -f srt -o meeting.srt
stt-cli transcribe meeting.m4a --dry-run --vad
```

생성된 전체 CLI 참조는 `stt-cli --help` 또는 각 하위 명령의 `--help`에서
확인할 수 있습니다.

## 자세한 문서

- [사용법](USAGE.md) — 전체 명령, 설정, 출력 형식, 작업 예제, 문제 해결
- [아키텍처](ARCHITECTURE.md) — 구성 요소, 데이터 흐름, 설계 결정, 현재 상태,
  확장 가이드

## 개발

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

제공자, 출력 형식, 타임스탬프 패턴, 릴리즈 경로를 추가하기 전에
[아키텍처](ARCHITECTURE.md#development-workflow)를 참고하세요.

## 라이선스

현재 저장소에는 `LICENSE` 파일이 없습니다. 현재의 비공개 범위 밖으로 배포하기
전에 사용할 라이선스를 확정하고 파일을 추가해야 합니다.
