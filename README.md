<div align="center">

# ⚡ BUTION

**Распределённый запуск больших LLM в локальной сети на базе `llama.cpp RPC`**

*Объединяйте оперативную память и вычислительные мощности нескольких компьютеров для запуска тяжёлых GGUF-моделей — без облаков, подписок и сложной настройки.*

[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-3b82f6?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/danytebyya/bution)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-green?style=for-the-badge)](LICENSE)
[![Engine](https://img.shields.io/badge/engine-llama.cpp%20RPC-blueviolet?style=for-the-badge)](https://github.com/ggml-org/llama.cpp)

---

</div>

## 💡 О проекте

**BUTION** решает проблему нехватки памяти (RAM/VRAM) на одном устройстве. 

Если для запуска 70B или 32B модели требуется 24–40 ГБ памяти, а у вас есть MacBook (16 ГБ) и Windows-ПК (16 ГБ), BUTION объединяет их в единый кластер. Веса модели распределяются по локальной сети, вычисления распараллеливаются через официальный `llama.cpp RPC`, а вы получаете стриминг ответов прямо в терминале.

> [!NOTE]
> GGUF-модель хранится **только на основном компьютере (Main)**. На второй компьютер (Worker) файл модели копировать не нужно — необходимые тензоры передаются на worker на лету по локальной сети.

---

## 📥 Быстрая установка

### 🍏 macOS (Apple Silicon / Intel)
```bash
curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | bash
```

### 🪟 Windows (x64)
```powershell
irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex
```

*Установщик автоматически загружает актуальные бинарники BUTION и `llama.cpp`, добавляет команду `bution` в PATH и настраивает локальные правила сети.*

---

## 🏁 Быстрый старт

### 1. Подключите устройства к одной сети
Подключите оба компьютера к одному Wi-Fi роутеру или соедините их напрямую кабелем Ethernet / Thunderbolt для максимальной скорости.

### 2. Запустите дополнительный компьютер (Worker)
На компьютере, где модели нет:
```bash
bution --role worker
```
*Оставьте это окно открытым. Узел перейдёт в режим ожидания подключения.*

### 3. Запустите основной компьютер (Main)
GGUF-модель должна находиться только на нём.

**macOS:**
```bash
bution --role main --model "/Users/username/Models/model.gguf"
```

**Windows:**
```powershell
bution --role main --model "D:\Models\model.gguf"
```

> [!TIP]
> Путь обязательно заключайте в кавычки, особенно если в нём есть пробелы.  
> Выбранная роль и путь к модели **автоматически сохраняются**. В следующие разы достаточно просто запустить:
> ```bash
> bution
> ```

### 4. Соедините компьютеры (Pairing)
После запуска BUTION автоматически обнаружит узлы в локальной сети:
1. На обоих экранах появится окно **Pairing** с 6-значным кодом подтверждения.
2. Сверьте код и нажмите **Accept** (`Enter`).
3. BUTION автоматически протестирует пропускную способность сети, распределит тензоры и запустит кластер.
4. Перейдите на вкладку **Chat** (`→`) и начните общение с моделью!

---

## ⚡ Прямое соединение через Ethernet-кабель (Turbo)

Для максимальной скорости генерации токенов (tok/s) рекомендуется соединить два компьютера напрямую сетевым кабелем Ethernet (или Thunderbolt/Type-C).

BUTION автоматически измерит скорость всех доступных интерфейсов и направит трафик тензоров через самый быстрый канал.

> [!TIP]
> Если IP-адреса для прямого Ethernet-соединения не назначились автоматически:
> - **Компьютер 1**: IP `192.168.50.1`, маска `255.255.255.0`
> - **Компьютер 2**: IP `192.168.50.2`, маска `255.255.255.0`
> - Поля *Шлюз (Gateway)* и *DNS* оставьте пустыми.

---

## ⌨️ Управление в TUI

| Клавиша | Действие |
|:---|:---|
| `Space` / `R` | Быстрое переключение роли узла (`Main` ⇄ `Worker` ⇄ `Auto`) |
| `Enter` | Запуск / остановка модели, подтверждение сопряжения, отправка в чат |
| `←` / `→` | Переключение вкладок (`Cluster`, `Nodes`, `Model`, `Benchmark`, `Chat`, `Settings`) |
| `Esc` | Отмена / отклонение входящего запроса сопряжения |
| `Ctrl + N` | Начать новый диалог в чате |
| `Ctrl + L` | Очистить историю сообщений |
| `Q` | Выход и корректная остановка всех фоновых процессов |

---

## 🛡 Безопасность и порты

| Порт | Протокол | Назначение |
|:---|:---|:---|
| `5353` | **UDP** | Обнаружение узлов в локальной сети (mDNS / Bonjour) |
| `31750` | **TCP** | Зашифрованный управляющий канал BUTION (`Noise XX`) |
| `31751` | **TCP** | Временный бенчмарк сети (открывается только после сопряжения) |
| `50052` | **TCP** | Передача тензоров `llama.cpp RPC` |
| `8080` | **TCP** | Внутренний HTTP API (слушает только `127.0.0.1` на Main) |

- Управляющий канал защищён шифрованием `Noise_XX_25519_ChaChaPoly_BLAKE2s`.
- Запуск RPC-сервера разрешается строго после обоюдного подтверждения через PIN-код.

---

## 🔄 Обновление

**macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/danytebyya/bution/main/install.sh | BUTION_FORCE_UPDATE=1 bash
```

**Windows:**
```powershell
$env:BUTION_FORCE_UPDATE="1"; irm https://raw.githubusercontent.com/danytebyya/bution/main/install.ps1 | iex
```

---

## 🛠 Сборка из исходников

```bash
git clone https://github.com/danytebyya/bution.git
cd bution
cargo test
cargo build --release
```

---

## 📄 Лицензия

Распространяется по лицензиям **MIT** или **Apache-2.0**. Подробности в файле [LICENSE](LICENSE).

