# BUTION

Запуск GGUF-моделей локально или на двух компьютерах через `llama.cpp RPC`, с чатом в терминале.

[Скачать последнюю версию](https://github.com/danytebyya/bution/releases/latest) · [Сообщить об ошибке](https://github.com/danytebyya/bution/issues) · [MIT](LICENSE)

## Быстрый старт

### 1. Установите BUTION

**Windows x64 — PowerShell 5.1 или новее:**

```powershell
irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex
```

**macOS — Apple Silicon или Intel:**

```bash
curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | bash
```

Установщик загрузит BUTION и `llama.cpp`, добавит команду `bution` в PATH.
На Windows подтвердите запрос на настройку Firewall. Затем откройте новое окно терминала.
Повторная установка пропускает уже установленные компоненты.

### 2. Выберите модель

Скачайте файл `.gguf` отдельно и укажите его полный путь. Для запуска на одном компьютере:

```text
bution --model "/полный/путь/model.gguf"
```

В Windows путь выглядит, например, так: `"D:\Models\model.gguf"`.

### 3. Запустите чат

На экране `Cluster` нажмите `Enter`, дождитесь загрузки модели и перейдите в `Chat` клавишей `Tab`.
Введите сообщение и нажмите `Enter`. Повторное нажатие `Enter` в `Cluster` останавливает модель.

Путь к модели сохраняется. Для следующего запуска достаточно команды `bution`.

## Подключение второго компьютера

Установите BUTION на оба компьютера и подключите их к одной доверенной локальной сети.
Файл модели нужен только на основном компьютере.

На дополнительном компьютере (worker):

```text
bution --role worker
```

На основном компьютере:

```text
bution --role main --model "/полный/путь/model.gguf"
```

1. В появившемся окне `Pairing` проверьте имя и адрес узла, выберите `Accept` и нажмите `Enter`.
2. Дождитесь результатов проверки сети в `Benchmark`.
3. На основном компьютере нажмите `Enter` в `Cluster`, затем перейдите в `Chat`.

Оставляйте BUTION открытым на обоих устройствах. Текущая версия использует один worker
и подключает его для вычислений, только если модель не помещается в доступную память основного компьютера.
Распределение отображается в `Cluster` → `DISTRIBUTION`.

Если узлы не находятся, проверьте, что сеть Windows имеет профиль «Частная» и роутер не изолирует устройства.

## Скачать архив

Прямые ссылки на последний опубликованный релиз:

- [Windows x64 — ZIP](https://github.com/danytebyya/bution/releases/latest/download/bution-windows-x64.zip)
- [macOS Apple Silicon — TAR.GZ](https://github.com/danytebyya/bution/releases/latest/download/bution-macos-arm64.tar.gz)
- [macOS Intel — TAR.GZ](https://github.com/danytebyya/bution/releases/latest/download/bution-macos-x64.tar.gz)

Архивы содержат только BUTION. Для автоматической настройки `llama.cpp` и PATH используйте установку выше.

## Управление

| Клавиша | Действие |
| --- | --- |
| `Tab` / `Shift+Tab` | Следующий / предыдущий экран |
| `Enter` | Запуск или остановка в `Cluster`, отправка сообщения в `Chat` |
| `Ctrl+N` / `Ctrl+L` | Очистить диалог в `Chat` |
| `Q` | Выход вне `Chat`; в чате — `Ctrl+Q` |

## Обновление

```text
bution --update
```

Версия: `bution --version`. Все параметры: `bution --help`.

## Разработка

Требуется Rust 1.85 или новее. Для запуска модели также нужны `llama-server`, `rpc-server` и `llama-bench`;
каталог с ними можно указать через `--llama-bin-dir`.

```bash
git clone https://github.com/danytebyya/bution.git
cd bution
cargo test --locked
cargo build --release --locked
```

[Архитектура](docs/ARCHITECTURE.md) · [Безопасность](docs/SECURITY.md) · [Лицензия MIT](LICENSE)
