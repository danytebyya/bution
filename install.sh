#!/usr/bin/env bash
set -euo pipefail

repo="danytebyya/bution"
install_dir="${BUTION_HOME:-${HOME}/.bution}"
bin_dir="${install_dir}/bin"
llama_dir="${install_dir}/llama"
force_update="${BUTION_FORCE_UPDATE:-0}"
temporary_dir="$(mktemp -d)"
trap 'rm -rf "${temporary_dir}"' EXIT

banner() {
  printf '\033[1;36m'
  cat <<'EOF'
  ____  _   _ _____ ___ ___  _   _ 
 | __ )| | | |_   _|_ _/ _ \| \ | |
 |  _ \| | | | | |  | | | | |  \| |
 | |_) | |_| | | |  | | |_| | |\  |
 |____/ \___/  |_| |___\___/|_| \_|
EOF
  printf '\033[0m\n'
  printf ' \033[1;33m⚡ BUTION\033[0m — \033[1;37mраспределённый запуск LLM в локальной сети\033[0m\n'
  printf '\033[90m─────────────────────────────────────────────────────────────\033[0m\n\n'
}

step() {
  printf '\033[1;34m[%s]\033[0m \033[1;37m%s\033[0m\n' "$1" "$2"
}

success() {
  printf '       \033[1;32m✔\033[0m \033[37m%s\033[0m\n' "$1"
}

info() {
  printf '       \033[36mℹ\033[0m \033[90m%s\033[0m\n' "$1"
}

fail() {
  printf '\n\033[1;31m✖ Ошибка:\033[0m %s\n' "$1" >&2
  exit 1
}

spin() {
  local pid=$1
  local msg="$2"
  local delay=0.08
  local spinstr="⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
  if [ -t 1 ]; then
    local i=0
    while kill -0 "$pid" 2>/dev/null; do
      i=$(( (i + 1) % 10 ))
      printf "\r       \033[36m%s\033[0m \033[90m%s\033[0m" "${spinstr:$i:1}" "$msg"
      sleep $delay
    done
    printf "\r\033[K"
  else
    wait "$pid"
  fi
}

download_file() {
  local url="$1"
  local dest="$2"
  if [ -t 1 ]; then
    curl -# -fL --retry 3 "${url}" -o "${dest}"
  else
    curl -sSfL --retry 3 "${url}" -o "${dest}"
  fi
}

banner

[[ "$(uname -s)" == "Darwin" ]] || fail "Этот установщик предназначен для macOS."
architecture="$(uname -m)"
case "${architecture}" in
  arm64) bution_asset="bution-macos-arm64.tar.gz"; llama_platform="macos-arm64" ;;
  x86_64) bution_asset="bution-macos-x64.tar.gz"; llama_platform="macos-x64" ;;
  *) fail "Архитектура ${architecture} пока не поддерживается." ;;
esac

command -v curl >/dev/null || fail "В macOS не найден curl."
mkdir -p "${bin_dir}" "${llama_dir}"

# 1/3: BUTION
step "1/3" "Загрузка BUTION…"
if [[ "${force_update}" != "1" && -x "${bin_dir}/bution-real" ]]; then
  success "BUTION уже установлен (${bin_dir}/bution-real)"
else
  bution_url="https://github.com/${repo}/releases/latest/download/${bution_asset}"
  bution_archive="${temporary_dir}/bution.tar.gz"
  bution_ok=0

  if download_file "${bution_url}" "${bution_archive}" 2>/dev/null; then
    tar -xzf "${bution_archive}" -C "${temporary_dir}" &
    spin $! "Распаковка архива BUTION…"
    wait $!
    cp "${temporary_dir}/bution" "${bin_dir}/bution-real"
    chmod +x "${bin_dir}/bution-real"
    success "BUTION успешно загружен и установлен"
    bution_ok=1
  fi

  if [[ "${bution_ok}" -eq 0 ]]; then
    info "Готовый релиз недоступен — выполняю автоматическую сборку…"
    if ! xcode-select -p >/dev/null 2>&1; then
      xcode-select --install >/dev/null 2>&1 || true
      fail "Подтвердите установку Apple Command Line Tools и повторите эту же команду."
    fi
    if ! command -v cargo >/dev/null 2>&1; then
      info "Установка Rust…"
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
      # shellcheck disable=SC1090
      source "${HOME}/.cargo/env"
    fi
    curl -fL --retry 3 "https://github.com/${repo}/archive/refs/heads/main.tar.gz" \
      -o "${temporary_dir}/source.tar.gz"
    mkdir "${temporary_dir}/source"
    tar -xzf "${temporary_dir}/source.tar.gz" -C "${temporary_dir}/source" --strip-components=1
    info "Компиляция BUTION (release)…"
    cargo build --release --locked --manifest-path "${temporary_dir}/source/Cargo.toml"
    cp "${temporary_dir}/source/target/release/bution" "${bin_dir}/bution-real"
    chmod +x "${bin_dir}/bution-real"
    success "BUTION успешно собран из исходников"
  fi
fi

echo ""

# 2/3: llama.cpp
llama_ready() {
  [[ -x "${llama_dir}/llama-server" ]] &&
    [[ -x "${llama_dir}/llama-bench" ]] &&
    [[ -x "${llama_dir}/rpc-server" || -x "${llama_dir}/ggml-rpc-server" ]]
}

step "2/3" "Загрузка llama.cpp с RPC…"
if [[ "${force_update}" != "1" ]] && llama_ready; then
  version="актуальная версия"
  [[ -f "${llama_dir}/.version" ]] && version="сборка $(cat "${llama_dir}/.version")"
  success "llama.cpp с RPC уже установлен (${version})"
else
  llama_tag="$(curl -fsSL --retry 3 \
    https://github.com/ggml-org/llama.cpp/releases/download/v0.3.0/nightly-tag.txt | tr -d '\r\n')"
  [[ "${llama_tag}" =~ ^b[0-9]+$ ]] || fail "Не удалось определить актуальную сборку llama.cpp: '${llama_tag}'."
  
  info "Актуальная сборка llama.cpp: ${llama_tag}"
  llama_asset="llama-${llama_tag}-bin-${llama_platform}.tar.gz"
  llama_url="https://github.com/ggml-org/llama.cpp/releases/download/${llama_tag}/${llama_asset}"
  llama_archive="${temporary_dir}/llama.tar.gz"
  
  download_file "${llama_url}" "${llama_archive}"
  mkdir "${temporary_dir}/llama"
  tar -xzf "${llama_archive}" -C "${temporary_dir}/llama" &
  spin $! "Распаковка архива llama.cpp…"
  wait $!

  llama_server="$(find "${temporary_dir}/llama" -type f -name 'llama-server' -print -quit)"
  [[ -n "${llama_server}" ]] || fail "В архиве llama.cpp отсутствует llama-server."
  llama_bin_source="$(dirname "${llama_server}")"
  rm -rf "${llama_dir:?}/"*
  cp -R "${llama_bin_source}/." "${llama_dir}/"
  chmod +x "${llama_dir}/"* 2>/dev/null || true
  printf '%s\n' "${llama_tag}" > "${llama_dir}/.version"
  llama_ready || fail "В официальном архиве отсутствуют компоненты llama.cpp RPC."
  success "llama.cpp с RPC успешно установлен (сборка ${llama_tag})"
fi

echo ""

# 3/3: Launcher and PATH
step "3/3" "Настройка лаунчера и переменной окружения PATH…"
printf '#!/bin/sh\nexec "%s" --llama-bin-dir "%s" "$@"\n' \
  "${bin_dir}/bution-real" "${llama_dir}" > "${bin_dir}/bution"
chmod +x "${bin_dir}/bution"
success "Лаунчер bution создан"

path_line='export PATH="$HOME/.bution/bin:$PATH"'
updated_profiles=0
for profile in "${HOME}/.zprofile" "${HOME}/.zshrc" "${HOME}/.bash_profile" "${HOME}/.bashrc"; do
  if [[ -f "${profile}" || "${profile}" == *".zshrc"* || "${profile}" == *".zprofile"* ]]; then
    touch "${profile}"
    if ! grep -Fqx "${path_line}" "${profile}"; then
      printf '\n%s\n' "${path_line}" >> "${profile}"
      updated_profiles=$((updated_profiles + 1))
    fi
  fi
done
success "Команда bution добавлена в PATH"

printf '\n\033[90m─────────────────────────────────────────────────────────────\033[0m\n'
printf ' \033[1;32m✨ Установка успешно завершена!\033[0m\n\n'
printf ' \033[1;37mЗапустить прямо сейчас:\033[0m\n'
printf '   \033[1;36m%s/bution\033[0m\n\n' "${bin_dir}"
printf ' \033[1;37mИли откройте новое окно Terminal и выполните:\033[0m\n'
printf '   \033[1;33mbution\033[0m\n\n'
