#!/usr/bin/env bash
set -euo pipefail

repo="danytebyya/bution"
install_dir="${BUTION_HOME:-${HOME}/.bution}"
bin_dir="${install_dir}/bin"
llama_dir="${install_dir}/llama"
temporary_dir="$(mktemp -d)"
trap 'rm -rf "${temporary_dir}"' EXIT

say() { printf '\033[1;36mBUTION\033[0m %s\n' "$1"; }
fail() { printf '\033[1;31mОшибка:\033[0m %s\n' "$1" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || fail "Этот установщик предназначен для macOS."
architecture="$(uname -m)"
case "${architecture}" in
  arm64) bution_asset="bution-macos-arm64.tar.gz"; llama_pattern='bin-macos-arm64\.tar\.gz' ;;
  x86_64) bution_asset="bution-macos-x64.tar.gz"; llama_pattern='bin-macos-x64\.tar\.gz' ;;
  *) fail "Архитектура ${architecture} пока не поддерживается." ;;
esac

command -v curl >/dev/null || fail "В macOS не найден curl."
mkdir -p "${bin_dir}" "${llama_dir}"

say "Загружаю BUTION…"
bution_url="https://github.com/${repo}/releases/latest/download/${bution_asset}"
if curl -fL --retry 3 "${bution_url}" -o "${temporary_dir}/bution.tar.gz"; then
  tar -xzf "${temporary_dir}/bution.tar.gz" -C "${temporary_dir}"
  cp "${temporary_dir}/bution" "${bin_dir}/bution-real"
else
  say "Готовый релиз недоступен — собираю BUTION автоматически…"
  if ! xcode-select -p >/dev/null 2>&1; then
    xcode-select --install >/dev/null 2>&1 || true
    fail "Подтвердите установку Apple Command Line Tools и повторите эту же команду."
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1090
    source "${HOME}/.cargo/env"
  fi
  curl -fL --retry 3 "https://github.com/${repo}/archive/refs/heads/main.tar.gz" \
    -o "${temporary_dir}/source.tar.gz"
  mkdir "${temporary_dir}/source"
  tar -xzf "${temporary_dir}/source.tar.gz" -C "${temporary_dir}/source" --strip-components=1
  cargo build --release --manifest-path "${temporary_dir}/source/Cargo.toml"
  cp "${temporary_dir}/source/target/release/bution" "${bin_dir}/bution-real"
fi
chmod +x "${bin_dir}/bution-real"

say "Загружаю официальный llama.cpp с RPC…"
release_json="$(curl -fsSL --retry 3 https://api.github.com/repos/ggml-org/llama.cpp/releases/latest)"
llama_url="$(printf '%s' "${release_json}" | sed -nE "s|.*\"browser_download_url\": \"([^\"]*${llama_pattern})\".*|\1|p" | head -n 1)"
[[ -n "${llama_url}" ]] || fail "GitHub не вернул подходящий архив llama.cpp."
curl -fL --retry 3 "${llama_url}" -o "${temporary_dir}/llama.tar.gz"
mkdir "${temporary_dir}/llama"
tar -xzf "${temporary_dir}/llama.tar.gz" -C "${temporary_dir}/llama"
llama_server="$(find "${temporary_dir}/llama" -type f -name 'llama-server' -print -quit)"
[[ -n "${llama_server}" ]] || fail "В архиве llama.cpp отсутствует llama-server."
llama_bin_source="$(dirname "${llama_server}")"
rm -rf "${llama_dir:?}/"*
cp -R "${llama_bin_source}/." "${llama_dir}/"
chmod +x "${llama_dir}/"* 2>/dev/null || true
[[ -x "${llama_dir}/llama-server" ]] || fail "Не удалось установить llama-server."
[[ -x "${llama_dir}/rpc-server" || -x "${llama_dir}/ggml-rpc-server" ]] || \
  fail "В официальном архиве нет RPC server."
[[ -x "${llama_dir}/llama-bench" ]] || fail "В официальном архиве нет llama-bench."

printf '#!/bin/sh\nexec "%s" --llama-bin-dir "%s" "$@"\n' \
  "${bin_dir}/bution-real" "${llama_dir}" > "${bin_dir}/bution"
chmod +x "${bin_dir}/bution"

path_line='export PATH="$HOME/.bution/bin:$PATH"'
for profile in "${HOME}/.zprofile" "${HOME}/.zshrc"; do
  touch "${profile}"
  grep -Fqx "${path_line}" "${profile}" || printf '\n%s\n' "${path_line}" >> "${profile}"
done

say "Установка завершена."
printf '\nЗапустить сейчас:\n  %s/bution\n\n' "${bin_dir}"
printf 'В новом окне Terminal можно использовать просто:\n  bution\n'

