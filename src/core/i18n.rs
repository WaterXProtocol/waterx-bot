//! Hand-rolled, dependency-free internationalisation.
//!
//! Every user-facing string lives here, grouped one-function-per-message so a
//! translator sees all locales for a given message side by side. Language is
//! auto-detected from Telegram's `User.language_code` (an IETF tag like `en`,
//! `zh-Hans`, `pt-BR`) via [`Lang::from_user`]; there is no per-user setting
//! command and no DB column. Unknown / unsupported tags fall back to English.
//!
//! ## Per-user vs. shared messages
//!
//! Direct replies and callback toasts are rendered in the *acting* user's
//! language. Messages that are a single shared/edited post — the bet-game
//! board and the sell/buy listings — are rendered in their **creator's**
//! language (the game host, or the seller/buyer), because one message body is
//! shown to everyone. `Prediction` therefore stores a [`Lang`] (see `game.rs`).
//!
//! ## Adding a message
//!
//! Add a function below and fill in all 15 arms of `tr!`. For messages with
//! runtime values, leave `{placeholder}` tokens in every arm and substitute
//! with `.replace(...)`; the helper [`fill`] / chained `replace` keeps it
//! type-free. Keep emoji in place — they read the same in every language.

/// Supported locales. `En` is the default / fallback (see [`Lang::from_code`]).
///
/// `Hant` = Traditional Chinese (the bot's original language); `Hans` =
/// Simplified Chinese. Serialize/Deserialize + Default(En) are needed because
/// `Prediction` persists a `Lang` to SQLite as JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    #[default]
    En,
    Hant,
    Hans,
    Ja,
    Ko,
    Ru,
    Fr,
    Es,
    De,
    Vi,
    Id,
    Fil,
    Th,
    Nl,
    Tr,
    // Added later; message bodies fall back to English until translated, but
    // the picker label, auto-detect and command menus work immediately.
    Pt,
    Hi,
    Ar,
}

/// Pick the matching arm for `$lang`. Arms are listed in the fixed order
/// `en, hant, hans, ja, ko, ru, fr, es, de, vi, id, fil, th, nl, tr, pt, hi, ar`.
macro_rules! tr {
    // Fully-translated form: all 18 arms supplied.
    ($lang:expr;
        $en:literal, $hant:literal, $hans:literal, $ja:literal, $ko:literal,
        $ru:literal, $fr:literal, $es:literal, $de:literal, $vi:literal,
        $id:literal, $fil:literal, $th:literal, $nl:literal, $tr:literal,
        $pt:literal, $hi:literal, $ar:literal $(,)?
    ) => {
        match $lang {
            Lang::En => $en,
            Lang::Hant => $hant,
            Lang::Hans => $hans,
            Lang::Ja => $ja,
            Lang::Ko => $ko,
            Lang::Ru => $ru,
            Lang::Fr => $fr,
            Lang::Es => $es,
            Lang::De => $de,
            Lang::Vi => $vi,
            Lang::Id => $id,
            Lang::Fil => $fil,
            Lang::Th => $th,
            Lang::Nl => $nl,
            Lang::Tr => $tr,
            Lang::Pt => $pt,
            Lang::Hi => $hi,
            Lang::Ar => $ar,
        }
    };
}

impl Lang {
    /// Map an IETF language tag (case-insensitive, `-`/`_` separated) to a
    /// [`Lang`]. Chinese is split into Traditional vs. Simplified by region /
    /// script subtag; bare `zh` defaults to Simplified. Everything unknown
    /// falls back to [`Lang::En`].
    pub fn from_code(code: &str) -> Lang {
        let c = code.to_ascii_lowercase();
        if c.starts_with("zh") {
            // zh-Hant / zh-TW / zh-HK / zh-MO → Traditional; else Simplified.
            if c.contains("hant") || c.contains("tw") || c.contains("hk") || c.contains("mo") {
                return Lang::Hant;
            }
            return Lang::Hans;
        }
        let primary = c.split(['-', '_']).next().unwrap_or("");
        match primary {
            "en" => Lang::En,
            "ja" => Lang::Ja,
            "ko" => Lang::Ko,
            "ru" => Lang::Ru,
            "fr" => Lang::Fr,
            "es" => Lang::Es,
            "de" => Lang::De,
            "vi" => Lang::Vi,
            "id" => Lang::Id,
            "fil" | "tl" => Lang::Fil,
            "th" => Lang::Th,
            "nl" => Lang::Nl,
            "tr" => Lang::Tr,
            "pt" => Lang::Pt,
            "hi" => Lang::Hi,
            "ar" => Lang::Ar,
            _ => Lang::En,
        }
    }

    /// Resolve the locale for a Telegram user, defaulting to English when the
    /// client reports no `language_code`.
    pub fn from_user(u: &telexide::model::User) -> Lang {
        u.language_code
            .as_deref()
            .map(Lang::from_code)
            .unwrap_or(Lang::En)
    }

    /// The two-letter code used when registering a localized command menu via
    /// `setMyCommands`. Telegram only accepts ISO 639-1 here, so the
    /// Traditional/Simplified split collapses to a single `zh` menu (Simplified
    /// wins, being registered last in [`Lang::ALL`]) and Filipino uses `tl`.
    pub fn menu_code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Hant => "zh",
            Lang::Hans => "zh",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
            Lang::Ru => "ru",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::De => "de",
            Lang::Vi => "vi",
            Lang::Id => "id",
            Lang::Fil => "tl",
            Lang::Th => "th",
            Lang::Nl => "nl",
            Lang::Tr => "tr",
            Lang::Pt => "pt",
            Lang::Hi => "hi",
            Lang::Ar => "ar",
        }
    }

    /// Stable, unique code used to persist a user's chosen locale in the DB.
    /// Unlike [`Lang::menu_code`] this never collapses the Hant/Hans split, so
    /// it round-trips losslessly via [`Lang::from_store_code`].
    pub fn store_code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Hant => "hant",
            Lang::Hans => "hans",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
            Lang::Ru => "ru",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::De => "de",
            Lang::Vi => "vi",
            Lang::Id => "id",
            Lang::Fil => "fil",
            Lang::Th => "th",
            Lang::Nl => "nl",
            Lang::Tr => "tr",
            Lang::Pt => "pt",
            Lang::Hi => "hi",
            Lang::Ar => "ar",
        }
    }

    /// Parse a [`Lang::store_code`] back into a `Lang`. Returns `None` for the
    /// empty string ("not set") or anything unrecognised.
    pub fn from_store_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.store_code() == code)
    }

    /// Flag + endonym label shown on the `/start` language-picker buttons.
    pub fn native_label(self) -> &'static str {
        match self {
            Lang::En => "🇬🇧 English",
            Lang::Ru => "🇷🇺 Русский",
            Lang::Hans => "🇨🇳 简体中文",
            Lang::Hant => "🇹🇼 繁體中文",
            Lang::Ja => "🇯🇵 日本語",
            Lang::Ko => "🇰🇷 한국어",
            Lang::Es => "🇪🇸 Español",
            Lang::Pt => "🇵🇹 Português",
            Lang::Fr => "🇫🇷 Français",
            Lang::De => "🇩🇪 Deutsch",
            Lang::Nl => "🇳🇱 Nederlands",
            Lang::Hi => "🇮🇳 हिन्दी",
            Lang::Ar => "🇸🇦 العربية",
            Lang::Tr => "🇹🇷 Türkçe",
            Lang::Vi => "🇻🇳 Tiếng Việt",
            Lang::Id => "🇮🇩 Bahasa Indonesia",
            Lang::Fil => "🇵🇭 Filipino",
            Lang::Th => "🇹🇭 ไทย",
        }
    }

    /// Every locale. This is also the display order of the `/start` language
    /// picker (chunked two-per-row), so the sequence here is intentional:
    /// Español/Português share a row, and Hindi/Arabic form the row directly
    /// below Nederlands.
    pub const ALL: [Lang; 18] = [
        Lang::En,
        Lang::Ru,
        Lang::Hans,
        Lang::Hant,
        Lang::Ja,
        Lang::Ko,
        Lang::Es,
        Lang::Pt,
        Lang::Fr,
        Lang::De,
        Lang::Nl,
        Lang::Tr,
        Lang::Hi,
        Lang::Ar,
        Lang::Vi,
        Lang::Id,
        Lang::Fil,
        Lang::Th,
    ];
}

// ----------------------------------------------------------------------------
// Parameter-free messages
// ----------------------------------------------------------------------------

pub fn not_enough_money(l: Lang) -> &'static str {
    tr!(l;
        "Not enough money 😶", "錢不夠耶😶", "钱不够耶😶", "お金が足りないよ😶", "돈이 부족해😶",
        "Не хватает денег 😶", "Pas assez d'argent 😶", "No tienes suficiente dinero 😶", "Nicht genug Geld 😶", "Không đủ tiền 😶",
        "Uangnya kurang 😶", "Kulang ang pera 😶", "เงินไม่พอ 😶", "Niet genoeg geld 😶", "Para yetmiyor 😶",
        "Dinheiro insuficiente 😶", "पैसे कम हैं 😶", "المال غير كافٍ 😶")
}

pub fn messing_around(l: Lang) -> &'static str {
    tr!(l;
        "Are you messing around? 🤨", "來亂的嗎🤨", "来乱的吗🤨", "ふざけてるの？🤨", "장난쳐?🤨",
        "Хулиганишь? 🤨", "Tu rigoles ? 🤨", "¿Estás jugando? 🤨", "Veräppelst du mich? 🤨", "Đùa à? 🤨",
        "Bercanda ya? 🤨", "Nambobola ka? 🤨", "กวนป่วนหรือเปล่า 🤨", "Loop je te dollen? 🤨", "Dalga mı geçiyorsun? 🤨",
        "Está de brincadeira? 🤨", "मज़ाक कर रहे हो? 🤨", "هل تمزح؟ 🤨")
}

pub fn reply_to_send_fruit(l: Lang) -> &'static str {
    tr!(l;
        "Reply to someone's message to send fruit 😅", "回覆對方訊息來送水果呦😅", "回复对方消息来送水果哟😅", "相手のメッセージに返信してフルーツを送ってね😅", "상대 메시지에 답장해서 과일을 보내세요😅",
        "Ответьте на сообщение, чтобы отправить фрукт 😅", "Réponds à un message pour envoyer un fruit 😅", "Responde a un mensaje para enviar fruta 😅", "Antworte auf eine Nachricht, um Obst zu senden 😅", "Trả lời tin nhắn của người khác để gửi trái cây 😅",
        "Balas pesan seseorang untuk mengirim buah 😅", "Mag-reply sa mensahe para magpadala ng prutas 😅", "ตอบกลับข้อความเพื่อส่งผลไม้ 😅", "Reageer op een bericht om fruit te sturen 😅", "Meyve göndermek için bir mesaja yanıt ver 😅",
        "Responda à mensagem de alguém para enviar fruta 😅", "फल भेजने के लिए किसी के संदेश का जवाब दें 😅", "ردّ على رسالة أحدهم لإرسال فاكهة 😅")
}

pub fn bot_no_money(l: Lang) -> &'static str {
    tr!(l;
        "I don't need money 😎", "我不需要錢喔 😎", "我不需要钱哦 😎", "お金はいらないよ 😎", "난 돈 필요 없어 😎",
        "Мне деньги не нужны 😎", "Je n'ai pas besoin d'argent 😎", "No necesito dinero 😎", "Ich brauche kein Geld 😎", "Tôi không cần tiền đâu 😎",
        "Aku nggak butuh uang 😎", "Hindi ko kailangan ng pera 😎", "ฉันไม่ต้องการเงินหรอก 😎", "Ik heb geen geld nodig 😎", "Paraya ihtiyacım yok 😎",
        "Não preciso de dinheiro 😎", "मुझे पैसे की ज़रूरत नहीं 😎", "لا أحتاج المال 😎")
}

pub fn grab_envelope_title(l: Lang) -> &'static str {
    tr!(l;
        "Grab the red envelope!", "搶紅包囉！", "抢红包啦！", "紅包を取ろう！", "행운 봉투를 잡아라!",
        "Хватай красный конверт!", "Attrape l'enveloppe rouge !", "¡Atrapa el sobre rojo!", "Schnapp dir den roten Umschlag!", "Giành lì xì nào!",
        "Rebut amplop merahnya!", "Agawin ang red envelope!", "คว้าซองแดงเลย!", "Grijp de rode envelop!", "Kırmızı zarfı kap!",
        "Pegue o envelope vermelho!", "लाल लिफ़ाफ़ा पकड़ो!", "التقط المظروف الأحمر!")
}

pub fn claim_button(l: Lang) -> &'static str {
    tr!(l;
        "Claim 🧧", "領取🧧", "领取🧧", "受け取る🧧", "받기🧧",
        "Забрать 🧧", "Récupérer 🧧", "Reclamar 🧧", "Einsammeln 🧧", "Nhận 🧧",
        "Ambil 🧧", "Kunin 🧧", "รับ 🧧", "Pak 🧧", "Al 🧧",
        "Resgatar 🧧", "लें 🧧", "استلام 🧧")
}

pub fn loading(l: Lang) -> &'static str {
    tr!(l;
        "(loading…)", "(載入中…)", "(加载中…)", "(読み込み中…)", "(불러오는 중…)",
        "(загрузка…)", "(chargement…)", "(cargando…)", "(lädt…)", "(đang tải…)",
        "(memuat…)", "(naglo-load…)", "(กำลังโหลด…)", "(laden…)", "(yükleniyor…)",
        "(carregando…)", "(लोड हो रहा है…)", "(جارٍ التحميل…)")
}

pub fn service_paused(l: Lang) -> &'static str {
    tr!(l;
        "(service paused)", "(暫停服務)", "(暂停服务)", "(サービス停止中)", "(서비스 일시 중지)",
        "(сервис приостановлен)", "(service en pause)", "(servicio en pausa)", "(Dienst pausiert)", "(tạm dừng dịch vụ)",
        "(layanan dijeda)", "(naka-pause ang serbisyo)", "(หยุดให้บริการชั่วคราว)", "(dienst gepauzeerd)", "(hizmet duraklatıldı)",
        "(serviço pausado)", "(सेवा रुकी हुई है)", "(الخدمة متوقفة)")
}

pub fn im_back(l: Lang) -> &'static str {
    tr!(l;
        "I'm back 🙂", "我回來了🙂", "我回来了🙂", "戻ってきたよ🙂", "돌아왔어🙂",
        "Я вернулся 🙂", "Je suis de retour 🙂", "Ya volví 🙂", "Ich bin zurück 🙂", "Tôi quay lại rồi 🙂",
        "Aku kembali 🙂", "Bumalik na ako 🙂", "ฉันกลับมาแล้ว 🙂", "Ik ben terug 🙂", "Geri döndüm 🙂",
        "Voltei 🙂", "मैं वापस आ गई 🙂", "لقد عدت 🙂")
}

pub fn someone_took_it(l: Lang) -> &'static str {
    tr!(l;
        "Someone else grabbed it 🙁", "別人領走了🙁", "别人领走了🙁", "他の人に取られたよ🙁", "다른 사람이 가져갔어🙁",
        "Кто-то уже забрал 🙁", "Quelqu'un l'a déjà pris 🙁", "Alguien más lo tomó 🙁", "Jemand anderes hat es genommen 🙁", "Người khác đã nhận mất rồi 🙁",
        "Sudah diambil orang lain 🙁", "May ibang nakakuha na 🙁", "คนอื่นรับไปแล้ว 🙁", "Iemand anders heeft het gepakt 🙁", "Başkası kaptı 🙁",
        "Outra pessoa pegou 🙁", "किसी और ने ले लिया 🙁", "أخذها شخص آخر 🙁")
}

pub fn grabbed_it(l: Lang) -> &'static str {
    tr!(l;
        "Got it! 😁", "搶到啦😁", "抢到啦😁", "ゲットした😁", "받았다😁",
        "Поймал! 😁", "Attrapé ! 😁", "¡Lo tomé! 😁", "Erwischt! 😁", "Nhận được rồi! 😁",
        "Dapat! 😁", "Nakuha! 😁", "คว้าได้แล้ว 😁", "Gepakt! 😁", "Kaptım! 😁",
        "Peguei! 😁", "मिल गया! 😁", "حصلت عليها! 😁")
}

pub fn too_many_fruits(l: Lang) -> &'static str {
    tr!(l;
        "Too many fruits 😶", "水果太多囉😶", "水果太多啦😶", "フルーツが多すぎるよ😶", "과일이 너무 많아😶",
        "Слишком много фруктов 😶", "Trop de fruits 😶", "Demasiada fruta 😶", "Zu viel Obst 😶", "Quá nhiều trái cây 😶",
        "Buahnya terlalu banyak 😶", "Sobra nang prutas 😶", "ผลไม้เยอะเกินไป 😶", "Te veel fruit 😶", "Çok fazla meyve 😶",
        "Frutas demais 😶", "बहुत ज़्यादा फल 😶", "فاكهة كثيرة جدًا 😶")
}

pub fn db_error(l: Lang) -> &'static str {
    tr!(l;
        "Database error", "資料庫錯誤", "数据库错误", "データベースエラー", "데이터베이스 오류",
        "Ошибка базы данных", "Erreur de base de données", "Error de base de datos", "Datenbankfehler", "Lỗi cơ sở dữ liệu",
        "Kesalahan basis data", "Error sa database", "ฐานข้อมูลผิดพลาด", "Databasefout", "Veritabanı hatası",
        "Erro de banco de dados", "डेटाबेस त्रुटि", "خطأ في قاعدة البيانات")
}

pub fn prediction_invalid(l: Lang) -> &'static str {
    tr!(l;
        "This game is no longer valid", "賭局已失效", "赌局已失效", "この賭けは無効です", "이 게임은 만료됐어",
        "Игра больше недействительна", "Ce pari n'est plus valide", "Esta apuesta ya no es válida", "Diese Wette ist nicht mehr gültig", "Ván cược không còn hiệu lực",
        "Permainan sudah tidak berlaku", "Hindi na valid ang laro", "เกมนี้ใช้ไม่ได้แล้ว", "Dit spel is niet meer geldig", "Bu oyun artık geçerli değil",
        "Este jogo não é mais válido", "यह गेम अब मान्य नहीं है", "هذه اللعبة لم تعد صالحة")
}

pub fn not_host(l: Lang) -> &'static str {
    tr!(l;
        "You're not the host 😶", "你不是莊家😶", "你不是庄家😶", "あなたは親じゃないよ😶", "당신은 딜러가 아니에요😶",
        "Вы не ведущий 😶", "Tu n'es pas l'organisateur 😶", "No eres el anfitrión 😶", "Du bist nicht der Gastgeber 😶", "Bạn không phải nhà cái 😶",
        "Kamu bukan bandar 😶", "Hindi ikaw ang host 😶", "คุณไม่ใช่เจ้ามือ 😶", "Jij bent niet de spelleider 😶", "Ev sahibi sen değilsin 😶",
        "Você não é o anfitrião 😶", "आप होस्ट नहीं हैं 😶", "لست المُضيف 😶")
}

pub fn already_closed(l: Lang) -> &'static str {
    tr!(l;
        "Already closed", "已收盤", "已收盘", "締め切り済み", "이미 마감됨",
        "Уже закрыто", "Déjà clôturé", "Ya cerrado", "Bereits geschlossen", "Đã đóng",
        "Sudah ditutup", "Sarado na", "ปิดรับแล้ว", "Al gesloten", "Zaten kapandı",
        "Já encerrado", "पहले ही बंद हो चुका", "مغلق بالفعل")
}

pub fn close_prediction_toast(l: Lang) -> &'static str {
    tr!(l;
        "Game closed", "關閉賭局", "关闭赌局", "賭けを締め切りました", "베팅 마감",
        "Ставки закрыты", "Paris clôturés", "Apuestas cerradas", "Wetten geschlossen", "Đã đóng cược",
        "Taruhan ditutup", "Sarado na ang pusta", "ปิดรับเดิมพันแล้ว", "Inzetten gesloten", "Bahisler kapandı",
        "Apostas encerradas", "बेटिंग बंद", "أُغلق الرهان")
}

/// Host tapped `[close]` before the deadline they set — blocked; `{time}` is the
/// deadline in the host's timezone.
pub fn close_before_deadline(l: Lang, time: &str) -> String {
    tr!(l;
        "⏳ Can't close early — betting runs until {time}.", "⏳ 無法提前關閉 — 下注將持續到 {time}。", "⏳ 无法提前关闭 — 下注将持续到 {time}。", "⏳ 早めに締め切れないよ — {time} まで受付中。", "⏳ 미리 마감할 수 없어 — {time}까지 진행돼.",
        "⏳ Нельзя закрыть раньше — приём ставок до {time}.", "⏳ Fermeture anticipée impossible — paris jusqu'à {time}.", "⏳ No puedes cerrar antes — apuestas hasta {time}.", "⏳ Vorzeitiges Schließen nicht möglich — Wetten bis {time}.", "⏳ Không thể đóng sớm — nhận cược đến {time}.",
        "⏳ Tidak bisa tutup lebih awal — taruhan sampai {time}.", "⏳ Hindi pwedeng isara nang maaga — taya hanggang {time}.", "⏳ ปิดก่อนกำหนดไม่ได้ — เดิมพันถึง {time}", "⏳ Kan niet vroeg sluiten — wedden tot {time}.", "⏳ Erken kapatılamaz — bahisler {time}'a kadar.",
        "⏳ Não dá para fechar antes — apostas até {time}.", "⏳ समय से पहले बंद नहीं कर सकते — {time} तक दांव।", "⏳ لا يمكن الإغلاق مبكرًا — الرهان حتى {time}.")
    .replace("{time}", time)
}

pub fn bad_stake(l: Lang) -> &'static str {
    tr!(l;
        "Invalid stake amount", "下注額錯誤", "下注额错误", "賭け金が不正です", "베팅 금액이 잘못됐어",
        "Неверная ставка", "Mise invalide", "Cantidad de apuesta inválida", "Ungültiger Einsatz", "Số tiền cược không hợp lệ",
        "Jumlah taruhan tidak valid", "Mali ang halaga ng pusta", "จำนวนเดิมพันไม่ถูกต้อง", "Ongeldige inzet", "Geçersiz bahis miktarı",
        "Valor de aposta inválido", "अमान्य दांव राशि", "مبلغ رهان غير صالح")
}

pub fn bet_failed(l: Lang) -> &'static str {
    tr!(l;
        "Bet failed", "下注失敗", "下注失败", "賭けに失敗", "베팅 실패",
        "Ставка не удалась", "Échec du pari", "Apuesta fallida", "Wette fehlgeschlagen", "Đặt cược thất bại",
        "Taruhan gagal", "Nabigo ang pusta", "เดิมพันล้มเหลว", "Inzet mislukt", "Bahis başarısız",
        "Falha na aposta", "दांव विफल", "فشل الرهان")
}

pub fn bet_success(l: Lang) -> &'static str {
    tr!(l;
        "Bet placed", "下注成功", "下注成功", "賭け成功", "베팅 완료",
        "Ставка принята", "Pari placé", "Apuesta realizada", "Wette platziert", "Đặt cược thành công",
        "Taruhan berhasil", "Tagumpay ang pusta", "เดิมพันสำเร็จ", "Inzet geplaatst", "Bahis başarılı",
        "Aposta registrada", "दांव लगाया गया", "تم وضع الرهان")
}

pub fn not_closed_yet(l: Lang) -> &'static str {
    tr!(l;
        "Not closed yet", "尚未收盤", "尚未收盘", "まだ締め切っていません", "아직 마감 안 됨",
        "Ещё не закрыто", "Pas encore clôturé", "Aún no cerrado", "Noch nicht geschlossen", "Chưa đóng cược",
        "Belum ditutup", "Hindi pa sarado", "ยังไม่ปิดรับ", "Nog niet gesloten", "Henüz kapanmadı",
        "Ainda não encerrado", "अभी बंद नहीं हुआ", "لم يُغلق بعد")
}

pub fn settle_success(l: Lang) -> &'static str {
    tr!(l;
        "Settled", "結算成功", "结算成功", "精算完了", "정산 완료",
        "Расчёт выполнен", "Réglé", "Liquidado", "Abgerechnet", "Đã thanh toán",
        "Selesai dihitung", "Naayos na", "ชำระเสร็จแล้ว", "Afgerekend", "Hesaplandı",
        "Liquidado", "निपटान पूरा", "تمت التسوية")
}

pub fn system_error(l: Lang) -> &'static str {
    tr!(l;
        "System error", "系統錯誤", "系统错误", "システムエラー", "시스템 오류",
        "Системная ошибка", "Erreur système", "Error del sistema", "Systemfehler", "Lỗi hệ thống",
        "Kesalahan sistem", "Error sa sistema", "ระบบผิดพลาด", "Systeemfout", "Sistem hatası",
        "Erro do sistema", "सिस्टम त्रुटि", "خطأ في النظام")
}

pub fn someone_dealt(l: Lang) -> &'static str {
    tr!(l;
        "Someone else closed the deal 🙁", "別人成交了🙁", "别人成交了🙁", "他の人が成立させたよ🙁", "다른 사람이 거래했어🙁",
        "Кто-то уже заключил сделку 🙁", "Quelqu'un a déjà conclu 🙁", "Alguien más cerró el trato 🙁", "Jemand anderes hat den Handel gemacht 🙁", "Người khác đã chốt rồi 🙁",
        "Sudah ditransaksikan orang lain 🙁", "May ibang nakatransaksyon na 🙁", "คนอื่นปิดดีลไปแล้ว 🙁", "Iemand anders heeft de deal gesloten 🙁", "Başkası anlaşmayı yaptı 🙁",
        "Outra pessoa fechou o negócio 🙁", "किसी और ने सौदा पूरा कर लिया 🙁", "أتمّ شخص آخر الصفقة 🙁")
}

pub fn withdrew_sell(l: Lang) -> &'static str {
    tr!(l;
        "Sell offer withdrawn", "已撤回賣單", "已撤回卖单", "売り注文を取り消しました", "판매 주문을 철회했어",
        "Предложение о продаже отозвано", "Offre de vente retirée", "Oferta de venta retirada", "Verkaufsangebot zurückgezogen", "Đã hủy lệnh bán",
        "Penawaran jual dibatalkan", "Binawi ang alok na benta", "ยกเลิกคำสั่งขายแล้ว", "Verkoopaanbod ingetrokken", "Satış teklifi geri çekildi",
        "Oferta de venda retirada", "बिक्री प्रस्ताव वापस लिया", "تم سحب عرض البيع")
}

pub fn withdrew_buy(l: Lang) -> &'static str {
    tr!(l;
        "Buy offer withdrawn", "已撤回買單", "已撤回买单", "買い注文を取り消しました", "구매 주문을 철회했어",
        "Предложение о покупке отозвано", "Offre d'achat retirée", "Oferta de compra retirada", "Kaufangebot zurückgezogen", "Đã hủy lệnh mua",
        "Penawaran beli dibatalkan", "Binawi ang alok na bili", "ยกเลิกคำสั่งซื้อแล้ว", "Aankoopaanbod ingetrokken", "Alış teklifi geri çekildi",
        "Oferta de compra retirada", "खरीद प्रस्ताव वापस लिया", "تم سحب عرض الشراء")
}

pub fn buyer_fruit_full(l: Lang) -> &'static str {
    tr!(l;
        "The buyer has too many fruits 😶", "買家水果太多囉😶", "买家水果太多啦😶", "買い手のフルーツが多すぎます😶", "구매자의 과일이 너무 많아😶",
        "У покупателя слишком много фруктов 😶", "L'acheteur a trop de fruits 😶", "El comprador tiene demasiada fruta 😶", "Der Käufer hat zu viel Obst 😶", "Người mua có quá nhiều trái cây 😶",
        "Pembeli punya terlalu banyak buah 😶", "Sobra nang prutas ng bumibili 😶", "ผู้ซื้อมีผลไม้มากเกินไป 😶", "De koper heeft te veel fruit 😶", "Alıcının çok fazla meyvesi var 😶",
        "O comprador tem frutas demais 😶", "खरीदार के पास बहुत ज़्यादा फल हैं 😶", "لدى المشتري فاكهة كثيرة جدًا 😶")
}

// --- /send-to-bot reaction lines (1..=5 fruits eaten) ---

pub fn eat_reaction(l: Lang, n: usize) -> &'static str {
    match n {
        1 => tr!(l;
            "Yummy", "好吃", "好吃", "おいしい", "맛있어",
            "Вкусно", "Délicieux", "Rico", "Lecker", "Ngon",
            "Enak", "Masarap", "อร่อย", "Lekker", "Lezzetli",
            "Gostoso", "स्वादिष्ट", "لذيذ"),
        2 => tr!(l;
            "So yummy", "好好吃", "好好吃", "とってもおいしい", "너무 맛있어",
            "Очень вкусно", "Trop bon", "Muy rico", "Sehr lecker", "Ngon quá",
            "Enak banget", "Ang sarap", "อร่อยมาก", "Heel lekker", "Çok lezzetli",
            "Muito gostoso", "बहुत स्वादिष्ट", "لذيذ جدًا"),
        3 => tr!(l;
            "So much!", "好多好多", "好多好多", "いっぱいだ", "엄청 많아",
            "Так много!", "Tellement !", "¡Cuánto!", "So viel!", "Nhiều quá!",
            "Banyak sekali!", "Ang dami!", "เยอะมาก!", "Zo veel!", "Çok fazla!",
            "Quanto!", "इतना सारा!", "كثير جدًا!"),
        4 => tr!(l;
            "So blissful", "好幸福", "好幸福", "幸せ", "너무 행복해",
            "Какое блаженство", "Quel bonheur", "Qué felicidad", "So glücklich", "Hạnh phúc quá",
            "Bahagia banget", "Sobrang saya", "มีความสุขมาก", "Zo gelukkig", "Çok mutluyum",
            "Que felicidade", "बहुत आनंद", "نعيم"),
        _ => tr!(l;
            "Blissed out to heaven", "幸福到升天", "幸福到升天", "天に昇る幸せ", "하늘로 승천할 만큼 행복해",
            "Блаженство до небес", "Au septième ciel", "En el séptimo cielo", "Im siebten Himmel", "Hạnh phúc thăng thiên",
            "Bahagia sampai ke langit", "Parang nasa langit ang saya", "มีความสุขจนลอยขึ้นสวรรค์", "In de zevende hemel", "Mutluluktan göklere uçuyorum",
            "Nas nuvens", "स्वर्ग जैसा आनंद", "في غاية السعادة"),
    }
}

// --- bet-game labels, buttons, verbs ---

pub fn state_betting(l: Lang) -> &'static str {
    tr!(l;
        "Betting open", "下注中", "下注中", "受付中", "베팅 중",
        "Приём ставок", "Paris ouverts", "Apuestas abiertas", "Wetten offen", "Đang nhận cược",
        "Sedang bertaruh", "Bukas ang pusta", "กำลังรับเดิมพัน", "Inzetten open", "Bahisler açık",
        "Apostas abertas", "बेटिंग चालू", "الرهان مفتوح")
}

pub fn state_closed(l: Lang) -> &'static str {
    tr!(l;
        "Closed", "已收盤", "已收盘", "締め切り", "마감",
        "Закрыто", "Clôturé", "Cerrado", "Geschlossen", "Đã đóng",
        "Ditutup", "Sarado", "ปิดรับ", "Gesloten", "Kapandı",
        "Encerrado", "बंद", "مغلق")
}

pub fn state_settled(l: Lang) -> &'static str {
    tr!(l;
        "Settled", "已結算", "已结算", "精算済み", "정산됨",
        "Рассчитано", "Réglé", "Liquidado", "Abgerechnet", "Đã thanh toán",
        "Selesai", "Naayos", "ชำระแล้ว", "Afgerekend", "Hesaplandı",
        "Liquidado", "निपटाया गया", "تمت التسوية")
}

pub fn draw_label(l: Lang) -> &'static str {
    tr!(l;
        "Draw", "平局", "平局", "引き分け", "무승부",
        "Ничья", "Nul", "Empate", "Unentschieden", "Hòa",
        "Seri", "Patas", "เสมอ", "Gelijkspel", "Berabere",
        "Empate", "ड्रॉ", "تعادل")
}

/// A self-host `/predict` settled as **void** — the result can't be determined,
/// so every stake is refunded (distinct from a sports `draw_label`).
pub fn void_label(l: Lang) -> &'static str {
    tr!(l;
        "Void", "流局", "流局", "流局", "무효",
        "Возврат", "Annulé", "Anulado", "Annulliert", "Hủy",
        "Batal", "Walang bisa", "ยกเลิก", "Geannuleerd", "İptal",
        "Anulado", "रद्द", "ملغى")
}

/// Section heading above the caller's open (unsettled) bets in `/status`.
pub fn positions_title(l: Lang) -> &'static str {
    tr!(l;
        "📊 Open bets", "📊 未結算下注", "📊 未结算下注", "📊 未決済のベット", "📊 미정산 베팅",
        "📊 Открытые ставки", "📊 Paris en cours", "📊 Apuestas abiertas", "📊 Offene Wetten", "📊 Cược đang mở",
        "📊 Taruhan terbuka", "📊 Bukas na taya", "📊 เดิมพันที่เปิดอยู่", "📊 Open weddenschappen", "📊 Açık bahisler",
        "📊 Apostas abertas", "📊 खुले दांव", "📊 الرهانات المفتوحة")
}

/// Section heading above the caller's open liquidity-provider stakes in `/assets`.
pub fn liquidity_title(l: Lang) -> &'static str {
    tr!(l;
        "💧 Liquidity provided", "💧 已提供流動性", "💧 已提供流动性", "💧 提供した流動性", "💧 제공한 유동성",
        "💧 Предоставленная ликвидность", "💧 Liquidité fournie", "💧 Liquidez aportada", "💧 Bereitgestellte Liquidität", "💧 Thanh khoản đã góp",
        "💧 Likuiditas disediakan", "💧 Liquidity na ibinigay", "💧 สภาพคล่องที่ให้ไว้", "💧 Verstrekte liquiditeit", "💧 Sağlanan likidite",
        "💧 Liquidez fornecida", "💧 दी गई लिक्विडिटी", "💧 السيولة المقدَّمة")
}

/// Section heading above the caller's stakes in still-open self-host
/// (`/predict`) games in `/status`.
pub fn predictions_title(l: Lang) -> &'static str {
    tr!(l;
        "🎲 Open predictions", "🎲 進行中的預測", "🎲 进行中的预测", "🎲 進行中の予測", "🎲 진행 중인 예측",
        "🎲 Активные прогнозы", "🎲 Prédictions en cours", "🎲 Predicciones abiertas", "🎲 Offene Vorhersagen", "🎲 Dự đoán đang mở",
        "🎲 Prediksi terbuka", "🎲 Bukás na prediksyon", "🎲 การทำนายที่เปิดอยู่", "🎲 Open voorspellingen", "🎲 Açık tahminler",
        "🎲 Previsões abertas", "🎲 चालू भविष्यवाणियाँ", "🎲 التوقعات المفتوحة")
}

/// Shown by `/bets` when the caller has no open market bets or predictions.
pub fn no_open_bets(l: Lang) -> &'static str {
    tr!(l;
        "You have no open bets 🫥", "你目前沒有未結算的下注 🫥", "你目前没有未结算的下注 🫥", "未決済のベットはないよ 🫥", "미정산 베팅이 없어 🫥",
        "У вас нет открытых ставок 🫥", "Tu n'as aucun pari en cours 🫥", "No tienes apuestas abiertas 🫥", "Du hast keine offenen Wetten 🫥", "Bạn chưa có cược nào đang mở 🫥",
        "Kamu belum punya taruhan terbuka 🫥", "Wala kang bukas na taya 🫥", "คุณยังไม่มีเดิมพันที่เปิดอยู่ 🫥", "Je hebt geen open weddenschappen 🫥", "Açık bahsin yok 🫥",
        "Você não tem apostas abertas 🫥", "आपके कोई खुले दांव नहीं हैं 🫥", "ليس لديك رهانات مفتوحة 🫥")
}

pub fn close_button(l: Lang) -> &'static str {
    tr!(l;
        "Close", "收盤", "收盘", "締め切る", "마감",
        "Закрыть", "Clôturer", "Cerrar", "Schließen", "Đóng",
        "Tutup", "Isara", "ปิดรับ", "Sluiten", "Kapat",
        "Encerrar", "बंद करें", "إغلاق")
}

pub fn verb_won(l: Lang) -> &'static str {
    tr!(l;
        "won", "贏了", "赢了", "勝った", "이김",
        "выиграл", "a gagné", "ganó", "gewann", "thắng",
        "menang", "nanalo", "ชนะ", "won", "kazandı",
        "ganhou", "जीते", "ربح")
}

pub fn verb_lost(l: Lang) -> &'static str {
    tr!(l;
        "lost", "輸了", "输了", "負けた", "잃음",
        "проиграл", "a perdu", "perdió", "verlor", "thua",
        "kalah", "natalo", "แพ้", "verloor", "kaybetti",
        "perdeu", "हारे", "خسر")
}

// ----------------------------------------------------------------------------
// Parameterised messages — leave {tokens} in every arm, substitute below.
// ----------------------------------------------------------------------------

pub fn sent_coins(l: Lang, sender: &str, recv: &str, coins: &str) -> String {
    tr!(l;
        "{sender} sent {recv}\n{coins} coins", "{sender} 送給 {recv}\n{coins} 顆 金幣", "{sender} 送给 {recv}\n{coins} 颗 金币", "{sender} が {recv} に\nコインを {coins} 枚 送った", "{sender} 님이 {recv} 님에게\n코인 {coins} 개 보냄",
        "{sender} отправил {recv}\n{coins} монет", "{sender} a envoyé à {recv}\n{coins} pièces", "{sender} envió a {recv}\n{coins} monedas", "{sender} hat {recv}\n{coins} Münzen geschickt", "{sender} đã gửi {recv}\n{coins} xu",
        "{sender} mengirim {recv}\n{coins} koin", "Nagpadala si {sender} kay {recv}\nng {coins} coins", "{sender} ส่งให้ {recv}\nเหรียญ {coins} เหรียญ", "{sender} stuurde {recv}\n{coins} munten", "{sender}, {recv} kullanıcısına\n{coins} para gönderdi",
        "{sender} enviou a {recv}\n{coins} moedas", "{sender} ने {recv} को\n{coins} कॉइन भेजे", "{sender} أرسل إلى {recv}\n{coins} عملة")
    .replace("{sender}", sender)
    .replace("{recv}", recv)
    .replace("{coins}", coins)
}

pub fn sent_envelope_title(l: Lang, sender: &str, coins: &str) -> String {
    tr!(l;
        "{sender} dropped a {coins} coin red envelope!", "{sender} 發紅包 {coins} 金幣！", "{sender} 发红包 {coins} 金币！", "{sender} が {coins} コインの紅包を配った！", "{sender} 님이 {coins} 코인 행운 봉투를 뿌렸어요!",
        "{sender} раздаёт красный конверт на {coins} монет!", "{sender} lâche une enveloppe rouge de {coins} pièces !", "¡{sender} soltó un sobre rojo de {coins} monedas!", "{sender} verteilt einen roten Umschlag mit {coins} Münzen!", "{sender} phát lì xì {coins} xu!",
        "{sender} membagikan amplop merah {coins} koin!", "Naghulog si {sender} ng red envelope na {coins} coins!", "{sender} แจกซองแดง {coins} เหรียญ!", "{sender} deelt een rode envelop van {coins} munten uit!", "{sender}, {coins} paralık kırmızı zarf bıraktı!",
        "{sender} soltou um envelope vermelho de {coins} moedas!", "{sender} ने {coins} कॉइन का लाल लिफ़ाफ़ा छोड़ा!", "{sender} أسقط مظروفًا أحمر بـ {coins} عملة!")
    .replace("{sender}", sender)
    .replace("{coins}", coins)
}

pub fn sent_fruits(l: Lang, sender: &str, recv: &str, fruits: &str) -> String {
    tr!(l;
        "{sender} sent {recv}\n{fruits}", "{sender} 送給 {recv}\n{fruits}", "{sender} 送给 {recv}\n{fruits}", "{sender} が {recv} に\n{fruits} を送った", "{sender} 님이 {recv} 님에게\n{fruits} 보냄",
        "{sender} отправил {recv}\n{fruits}", "{sender} a envoyé à {recv}\n{fruits}", "{sender} envió a {recv}\n{fruits}", "{sender} hat {recv}\n{fruits} geschickt", "{sender} đã gửi {recv}\n{fruits}",
        "{sender} mengirim {recv}\n{fruits}", "Nagpadala si {sender} kay {recv}\n{fruits}", "{sender} ส่งให้ {recv}\n{fruits}", "{sender} stuurde {recv}\n{fruits}", "{sender}, {recv} kullanıcısına\n{fruits} gönderdi",
        "{sender} enviou a {recv}\n{fruits}", "{sender} ने {recv} को\n{fruits} भेजा", "{sender} أرسل إلى {recv}\n{fruits}")
    .replace("{sender}", sender)
    .replace("{recv}", recv)
    .replace("{fruits}", fruits)
}

pub fn thanks(l: Lang, sender: &str, line: &str) -> String {
    tr!(l;
        "Thank you {sender}!\n{line}", "謝謝 {sender}！\n{line}", "谢谢 {sender}！\n{line}", "ありがとう {sender}！\n{line}", "고마워 {sender}!\n{line}",
        "Спасибо, {sender}!\n{line}", "Merci {sender} !\n{line}", "¡Gracias {sender}!\n{line}", "Danke {sender}!\n{line}", "Cảm ơn {sender}!\n{line}",
        "Terima kasih {sender}!\n{line}", "Salamat {sender}!\n{line}", "ขอบคุณ {sender}!\n{line}", "Bedankt {sender}!\n{line}", "Teşekkürler {sender}!\n{line}",
        "Obrigada {sender}!\n{line}", "धन्यवाद {sender}!\n{line}", "شكرًا {sender}!\n{line}")
    .replace("{sender}", sender)
    .replace("{line}", line)
}

pub fn sell_button(l: Lang, price: &str) -> String {
    tr!(l;
        "Buy for ${price}", "${price} 買入", "${price} 买入", "${price} で買う", "${price} 에 구매",
        "Купить за ${price}", "Acheter pour ${price}", "Comprar por ${price}", "Für ${price} kaufen", "Mua với giá ${price}",
        "Beli seharga ${price}", "Bilhin sa ${price}", "ซื้อในราคา ${price}", "Koop voor ${price}", "${price} karşılığında al",
        "Comprar por ${price}", "${price} में खरीदें", "اشترِ مقابل ${price}")
    .replace("{price}", price)
}

pub fn buy_button(l: Lang, price: &str) -> String {
    tr!(l;
        "Sell for ${price}", "${price} 賣出", "${price} 卖出", "${price} で売る", "${price} 에 판매",
        "Продать за ${price}", "Vendre pour ${price}", "Vender por ${price}", "Für ${price} verkaufen", "Bán với giá ${price}",
        "Jual seharga ${price}", "Ibenta sa ${price}", "ขายในราคา ${price}", "Verkoop voor ${price}", "${price} karşılığında sat",
        "Vender por ${price}", "${price} में बेचें", "بِع مقابل ${price}")
    .replace("{price}", price)
}

pub fn sell_listing(l: Lang, seller: &str, fruits: &str, price: &str) -> String {
    tr!(l;
        "{seller} is selling {fruits}\nasking {price} coins", "{seller} 出售 {fruits}\n要價 {price} 金幣", "{seller} 出售 {fruits}\n要价 {price} 金币", "{seller} が {fruits} を売り出し\n希望価格 {price} コイン", "{seller} 님이 {fruits} 판매\n희망가 {price} 코인",
        "{seller} продаёт {fruits}\nцена {price} монет", "{seller} vend {fruits}\nprix demandé {price} pièces", "{seller} vende {fruits}\npide {price} monedas", "{seller} verkauft {fruits}\nfür {price} Münzen", "{seller} đang bán {fruits}\ngiá {price} xu",
        "{seller} menjual {fruits}\nharga {price} koin", "Nagbebenta si {seller} ng {fruits}\nhinihingi {price} coins", "{seller} ขาย {fruits}\nราคา {price} เหรียญ", "{seller} verkoopt {fruits}\nvraagprijs {price} munten", "{seller}, {fruits} satıyor\nistenen fiyat {price} para",
        "{seller} está vendendo {fruits}\npedindo {price} moedas", "{seller} {fruits} बेच रहे हैं\nमाँग {price} कॉइन", "{seller} يبيع {fruits}\nالسعر المطلوب {price} عملة")
    .replace("{seller}", seller)
    .replace("{fruits}", fruits)
    .replace("{price}", price)
}

pub fn buy_listing(l: Lang, buyer: &str, fruits: &str, price: &str) -> String {
    tr!(l;
        "{buyer} wants to buy {fruits}\noffering {price} coins", "{buyer} 收購 {fruits}\n出價 {price} 金幣", "{buyer} 收购 {fruits}\n出价 {price} 金币", "{buyer} が {fruits} を買い取り\n提示額 {price} コイン", "{buyer} 님이 {fruits} 매입\n제시가 {price} 코인",
        "{buyer} скупает {fruits}\nпредлагает {price} монет", "{buyer} achète {fruits}\noffre {price} pièces", "{buyer} compra {fruits}\nofrece {price} monedas", "{buyer} kauft {fruits}\nbietet {price} Münzen", "{buyer} thu mua {fruits}\ntrả {price} xu",
        "{buyer} membeli {fruits}\nmenawar {price} koin", "Bumibili si {buyer} ng {fruits}\nnag-aalok ng {price} coins", "{buyer} รับซื้อ {fruits}\nเสนอ {price} เหรียญ", "{buyer} koopt {fruits}\nbiedt {price} munten", "{buyer}, {fruits} alıyor\n{price} para teklif ediyor",
        "{buyer} quer comprar {fruits}\noferecendo {price} moedas", "{buyer} {fruits} खरीदना चाहते हैं\nप्रस्ताव {price} कॉइन", "{buyer} يريد شراء {fruits}\nيعرض {price} عملة")
    .replace("{buyer}", buyer)
    .replace("{fruits}", fruits)
    .replace("{price}", price)
}

pub fn received_fruit(l: Lang, name: &str, fruit: &str) -> String {
    tr!(l;
        "🧧 {name} received a {fruit}", "🧧 {name} 收到一顆 {fruit}", "🧧 {name} 收到一颗 {fruit}", "🧧 {name} が {fruit} を1つ受け取った", "🧧 {name} 님이 {fruit} 한 개 받음",
        "🧧 {name} получил {fruit}", "🧧 {name} a reçu un {fruit}", "🧧 {name} recibió un {fruit}", "🧧 {name} hat ein {fruit} erhalten", "🧧 {name} nhận được một {fruit}",
        "🧧 {name} menerima sebuah {fruit}", "🧧 Nakatanggap si {name} ng {fruit}", "🧧 {name} ได้รับ {fruit} หนึ่งลูก", "🧧 {name} ontving een {fruit}", "🧧 {name} bir {fruit} aldı",
        "🧧 {name} recebeu um {fruit}", "🧧 {name} को एक {fruit} मिला", "🧧 {name} حصل على {fruit}")
    .replace("{name}", name)
    .replace("{fruit}", fruit)
}

pub fn received_coins(l: Lang, name: &str, coins: &str) -> String {
    tr!(l;
        "🧧 {name} received {coins} coins", "🧧 {name} 收到 {coins} 顆 金幣", "🧧 {name} 收到 {coins} 颗 金币", "🧧 {name} がコインを {coins} 枚 受け取った", "🧧 {name} 님이 코인 {coins} 개 받음",
        "🧧 {name} получил {coins} монет", "🧧 {name} a reçu {coins} pièces", "🧧 {name} recibió {coins} monedas", "🧧 {name} hat {coins} Münzen erhalten", "🧧 {name} nhận được {coins} xu",
        "🧧 {name} menerima {coins} koin", "🧧 Nakatanggap si {name} ng {coins} coins", "🧧 {name} ได้รับ {coins} เหรียญ", "🧧 {name} ontving {coins} munten", "🧧 {name}, {coins} para aldı",
        "🧧 {name} recebeu {coins} moedas", "🧧 {name} को {coins} कॉइन मिले", "🧧 {name} حصل على {coins} عملة")
    .replace("{name}", name)
    .replace("{coins}", coins)
}

/// Footer under the live betting board (pari-mutuel pool, still open).
pub fn board_footer_open(l: Lang, total: &str) -> String {
    tr!(l;
        "Pool {total} 🪙 · tap to bet (odds live)", "彩池 {total} 🪙 · 點選下注（賠率即時更新）", "彩池 {total} 🪙 · 点击下注（赔率实时更新）", "プール {total} 🪙 · タップで賭ける（オッズは随時変動）", "풀 {total} 🪙 · 탭하여 베팅 (배당 실시간)",
        "Банк {total} 🪙 · нажми, чтобы поставить (кэф меняется)", "Cagnotte {total} 🪙 · touche pour parier (cotes en direct)", "Bote {total} 🪙 · toca para apostar (cuotas en vivo)", "Pool {total} 🪙 · zum Wetten tippen (Quoten live)", "Quỹ {total} 🪙 · chạm để cược (tỷ lệ cập nhật)",
        "Pool {total} 🪙 · ketuk untuk bertaruh (odds langsung)", "Pool {total} 🪙 · i-tap para tumaya (live ang odds)", "พูล {total} 🪙 · แตะเพื่อเดิมพัน (อัตราต่อรองเรียลไทม์)", "Pot {total} 🪙 · tik om in te zetten (odds live)", "Havuz {total} 🪙 · bahis için dokun (oranlar canlı)",
        "Bolão {total} 🪙 · toque para apostar (odds ao vivo)", "पूल {total} 🪙 · दांव लगाने के लिए टैप करें (ऑड्स लाइव)", "المجمع {total} 🪙 · انقر للمراهنة (الاحتمالات مباشرة)")
    .replace("{total}", total)
}

/// Footer under the board once betting is closed (awaiting the host's result).
pub fn board_footer_closed(l: Lang, total: &str) -> String {
    tr!(l;
        "Pool {total} 🪙 · betting closed, awaiting result", "彩池 {total} 🪙 · 已截止，等待開獎", "彩池 {total} 🪙 · 已截止，等待开奖", "プール {total} 🪙 · 締め切り、結果待ち", "풀 {total} 🪙 · 마감, 결과 대기",
        "Банк {total} 🪙 · ставки закрыты, ждём результат", "Cagnotte {total} 🪙 · paris clos, en attente du résultat", "Bote {total} 🪙 · apuestas cerradas, esperando resultado", "Pool {total} 🪙 · Wetten geschlossen, Ergebnis ausstehend", "Quỹ {total} 🪙 · đã đóng cược, chờ kết quả",
        "Pool {total} 🪙 · taruhan ditutup, menunggu hasil", "Pool {total} 🪙 · sarado na ang taya, hinihintay ang resulta", "พูล {total} 🪙 · ปิดรับแล้ว รอผล", "Pot {total} 🪙 · inzetten gesloten, wacht op uitslag", "Havuz {total} 🪙 · bahisler kapandı, sonuç bekleniyor",
        "Bolão {total} 🪙 · apostas encerradas, aguardando resultado", "पूल {total} 🪙 · दांव बंद, परिणाम प्रतीक्षित", "المجمع {total} 🪙 · أُغلقت الرهانات، بانتظار النتيجة")
    .replace("{total}", total)
}

/// Private toast (only the tapper sees it) showing their accumulated, not-yet-
/// confirmed stake on an option in a shared `/predict` board.
pub fn bet_pending(l: Lang, amt: &str, opt: &str) -> String {
    tr!(l;
        "Pending: {amt} 🪙 on {opt}", "待確認：{amt} 🪙 押 {opt}", "待确认：{amt} 🪙 押 {opt}", "保留中：{opt} に {amt} 🪙", "대기 중: {opt} 에 {amt} 🪙",
        "Ожидает: {amt} 🪙 на {opt}", "En attente : {amt} 🪙 sur {opt}", "Pendiente: {amt} 🪙 a {opt}", "Ausstehend: {amt} 🪙 auf {opt}", "Đang chờ: {amt} 🪙 cho {opt}",
        "Tertunda: {amt} 🪙 pada {opt}", "Nakabinbin: {amt} 🪙 sa {opt}", "รอยืนยัน: {amt} 🪙 ที่ {opt}", "In afwachting: {amt} 🪙 op {opt}", "Beklemede: {opt} için {amt} 🪙",
        "Pendente: {amt} 🪙 em {opt}", "लंबित: {opt} पर {amt} 🪙", "معلّق: {amt} 🪙 على {opt}")
    .replace("{amt}", amt)
    .replace("{opt}", opt)
}

/// Private toast confirming the tapper's pending stake was cleared.
pub fn bet_cleared(l: Lang) -> &'static str {
    tr!(l;
        "Pending stake cleared", "已清除待確認下注", "已清除待确认下注", "保留中の賭けをクリアしました", "대기 중인 베팅을 지웠어요",
        "Ставка сброшена", "Mise en attente effacée", "Apuesta pendiente borrada", "Ausstehender Einsatz gelöscht", "Đã xóa cược đang chờ",
        "Taruhan tertunda dihapus", "Na-clear ang nakabinbing taya", "ล้างเดิมพันที่รอยืนยันแล้ว", "Openstaande inzet gewist", "Bekleyen bahis temizlendi",
        "Aposta pendente apagada", "लंबित दांव साफ़ किया गया", "تم مسح الرهان المعلّق")
}

pub fn bought_msg(l: Lang, name: &str, price: &str, fruits: &str) -> String {
    tr!(l;
        "{name} spent {price} coins\nand bought {fruits}", "{name} 花 {price} 金幣\n買了 {fruits}", "{name} 花 {price} 金币\n买了 {fruits}", "{name} がコインを {price} 枚 使って\n{fruits} を買った", "{name} 님이 코인 {price} 개 써서\n{fruits} 구매",
        "{name} потратил {price} монет\nи купил {fruits}", "{name} a dépensé {price} pièces\net acheté {fruits}", "{name} gastó {price} monedas\ny compró {fruits}", "{name} hat {price} Münzen ausgegeben\nund {fruits} gekauft", "{name} đã tiêu {price} xu\nvà mua {fruits}",
        "{name} menghabiskan {price} koin\ndan membeli {fruits}", "Gumastos si {name} ng {price} coins\nat bumili ng {fruits}", "{name} จ่าย {price} เหรียญ\nและซื้อ {fruits}", "{name} gaf {price} munten uit\nen kocht {fruits}", "{name}, {price} para harcayıp\n{fruits} aldı",
        "{name} gastou {price} moedas\ne comprou {fruits}", "{name} ने {price} कॉइन खर्च कर\n{fruits} खरीदा", "{name} أنفق {price} عملة\nواشترى {fruits}")
    .replace("{name}", name)
    .replace("{price}", price)
    .replace("{fruits}", fruits)
}

pub fn bought_toast(l: Lang, fruits: &str) -> String {
    tr!(l;
        "Bought {fruits} 🥳", "買了 {fruits}🥳", "买了 {fruits}🥳", "{fruits} を買った🥳", "{fruits} 샀다🥳",
        "Куплено {fruits} 🥳", "{fruits} acheté 🥳", "Compraste {fruits} 🥳", "{fruits} gekauft 🥳", "Đã mua {fruits} 🥳",
        "Beli {fruits} 🥳", "Nabili ang {fruits} 🥳", "ซื้อ {fruits} แล้ว 🥳", "{fruits} gekocht 🥳", "{fruits} alındı 🥳",
        "Comprou {fruits} 🥳", "{fruits} खरीदा 🥳", "تم شراء {fruits} 🥳")
    .replace("{fruits}", fruits)
}

pub fn sold_msg(l: Lang, name: &str, fruits: &str, price: &str) -> String {
    tr!(l;
        "{name} sold {fruits}\nand earned {price} coins", "{name} 賣出 {fruits}\n賺了 {price} 金幣", "{name} 卖出 {fruits}\n赚了 {price} 金币", "{name} が {fruits} を売って\nコインを {price} 枚 稼いだ", "{name} 님이 {fruits} 팔아서\n코인 {price} 개 벌었어요",
        "{name} продал {fruits}\nи заработал {price} монет", "{name} a vendu {fruits}\net gagné {price} pièces", "{name} vendió {fruits}\ny ganó {price} monedas", "{name} hat {fruits} verkauft\nund {price} Münzen verdient", "{name} đã bán {fruits}\nvà kiếm được {price} xu",
        "{name} menjual {fruits}\ndan mendapat {price} koin", "Ibinenta ni {name} ang {fruits}\nat kumita ng {price} coins", "{name} ขาย {fruits}\nและได้ {price} เหรียญ", "{name} verkocht {fruits}\nen verdiende {price} munten", "{name}, {fruits} satıp\n{price} para kazandı",
        "{name} vendeu {fruits}\ne ganhou {price} moedas", "{name} ने {fruits} बेचा\nऔर {price} कॉइन कमाए", "{name} باع {fruits}\nوربح {price} عملة")
    .replace("{name}", name)
    .replace("{fruits}", fruits)
    .replace("{price}", price)
}

pub fn sold_toast(l: Lang, price: &str) -> String {
    tr!(l;
        "Earned {price} coins 🥳", "賺取 {price} 金幣🥳", "赚取 {price} 金币🥳", "コインを {price} 枚 稼いだ🥳", "코인 {price} 개 벌었다🥳",
        "Заработано {price} монет 🥳", "{price} pièces gagnées 🥳", "Ganaste {price} monedas 🥳", "{price} Münzen verdient 🥳", "Kiếm được {price} xu 🥳",
        "Dapat {price} koin 🥳", "Kumita ng {price} coins 🥳", "ได้ {price} เหรียญ 🥳", "{price} munten verdiend 🥳", "{price} para kazanıldı 🥳",
        "Ganhou {price} moedas 🥳", "{price} कॉइन कमाए 🥳", "ربح {price} عملة 🥳")
    .replace("{price}", price)
}

pub fn you_dont_have(l: Lang, ch: &str) -> String {
    tr!(l;
        "You don't have {ch} 😶", "你沒有 {ch} 喔😶", "你没有 {ch} 哦😶", "{ch} を持っていないよ😶", "{ch} 이(가) 없어😶",
        "У вас нет {ch} 😶", "Tu n'as pas de {ch} 😶", "No tienes {ch} 😶", "Du hast kein {ch} 😶", "Bạn không có {ch} 😶",
        "Kamu nggak punya {ch} 😶", "Wala kang {ch} 😶", "คุณไม่มี {ch} 😶", "Je hebt geen {ch} 😶", "Sende {ch} yok 😶",
        "Você não tem {ch} 😶", "आपके पास {ch} नहीं है 😶", "ليس لديك {ch} 😶")
    .replace("{ch}", ch)
}

// --- bet-game settlement display (rendered in the host's language) ---

pub fn result_header(l: Lang, outcome: &str) -> String {
    tr!(l;
        "Host's result: {outcome}", "莊家指定結果：{outcome}", "庄家指定结果：{outcome}", "親の指定結果：{outcome}", "딜러 지정 결과: {outcome}",
        "Итог от ведущего: {outcome}", "Résultat de l'organisateur : {outcome}", "Resultado del anfitrión: {outcome}", "Ergebnis des Gastgebers: {outcome}", "Kết quả nhà cái: {outcome}",
        "Hasil dari bandar: {outcome}", "Resulta ng host: {outcome}", "ผลที่เจ้ามือกำหนด: {outcome}", "Uitslag van spelleider: {outcome}", "Ev sahibinin sonucu: {outcome}",
        "Resultado do anfitrião: {outcome}", "होस्ट का परिणाम: {outcome}", "نتيجة المُضيف: {outcome}")
    .replace("{outcome}", outcome)
}

/// Appended to the void header when the host settled an outcome **nobody bet** —
/// the pool is refunded rather than burned.
pub fn no_winners_refund(l: Lang) -> &'static str {
    tr!(l;
        "\nNo winning bets — all stakes refunded 🔄", "\n沒有人押中 — 全額退回 🔄", "\n没有人押中 — 全额退回 🔄", "\n的中者なし — 全額返金したよ 🔄", "\n맞힌 사람이 없어 — 전액 환불했어 🔄",
        "\nНет выигрышных ставок — всё возвращено 🔄", "\nAucun pari gagnant — tout est remboursé 🔄", "\nSin apuestas ganadoras — todo reembolsado 🔄", "\nKeine Gewinnwetten — alles erstattet 🔄", "\nKhông có cược thắng — đã hoàn lại tất cả 🔄",
        "\nTidak ada taruhan menang — semua dikembalikan 🔄", "\nWalang nanalong taya — isinauli lahat 🔄", "\nไม่มีใครเดิมพันถูก — คืนเงินทั้งหมด 🔄", "\nGeen winnende inzetten — alles terugbetaald 🔄", "\nKazanan bahis yok — tümü iade edildi 🔄",
        "\nSem apostas vencedoras — tudo reembolsado 🔄", "\nकोई जीतने वाला दांव नहीं — सब वापस 🔄", "\nلا رهانات فائزة — تم رد الجميع 🔄")
}

/// Shown when a prediction settles with payouts but **no net winner** (all the
/// money was on the winning side, so everyone just gets their stake back).
pub fn all_broke_even(l: Lang) -> &'static str {
    tr!(l;
        "\nEveryone broke even — stakes returned 🤝", "\n大家打平 — 退回本金 🤝", "\n大家打平 — 退回本金 🤝", "\nみんな引き分け — 元金を返したよ 🤝", "\n모두 본전 — 베팅금 돌려줬어 🤝",
        "\nВсе при своих — ставки возвращены 🤝", "\nÉgalité pour tous — mises rendues 🤝", "\nTodos en tablas — apuestas devueltas 🤝", "\nAlle unentschieden — Einsätze zurück 🤝", "\nHòa cả làng — đã trả lại tiền cược 🤝",
        "\nSemua impas — taruhan dikembalikan 🤝", "\nPatas lahat — isinauli ang taya 🤝", "\nเสมอกันทุกคน — คืนเงินเดิมพัน 🤝", "\nIedereen quitte — inzetten terug 🤝", "\nHerkes başa baş — bahisler iade 🤝",
        "\nTodos empataram — apostas devolvidas 🤝", "\nसब बराबर — दांव वापस 🤝", "\nالجميع تعادل — أُعيدت الرهانات 🤝")
}

pub fn no_one_bet_suffix(l: Lang) -> &'static str {
    tr!(l;
        "\nbut it seems nobody placed a bet 😶", "\n但好像沒有人下注欸😶", "\n但好像没有人下注欸😶", "\nでも誰も賭けてないみたい😶", "\n근데 아무도 안 건 것 같아😶",
        "\nно, похоже, никто не сделал ставку 😶", "\nmais on dirait que personne n'a parié 😶", "\npero parece que nadie apostó 😶", "\naber anscheinend hat niemand gewettet 😶", "\nnhưng hình như không ai đặt cược 😶",
        "\ntapi sepertinya tidak ada yang bertaruh 😶", "\npero mukhang walang nagtaya 😶", "\nแต่เหมือนจะไม่มีใครเดิมพันเลย 😶", "\nmaar het lijkt erop dat niemand heeft ingezet 😶", "\nama görünüşe göre kimse bahis oynamadı 😶",
        "\nmas parece que ninguém apostou 😶", "\nलेकिन लगता है किसी ने दांव नहीं लगाया 😶", "\nلكن يبدو أن لا أحد راهن 😶")
}

// ----------------------------------------------------------------------------
// Command-menu descriptions (the "/" autocomplete). Registered per-language
// via `setMyCommands` in `bot::run`.
// ----------------------------------------------------------------------------

/// `(command, description)` pairs for the bot's command menu in `l`. Order and
/// command names must match the `create_framework!` list in `bot.rs`.
pub fn command_menu(l: Lang) -> [(&'static str, &'static str); 10] {
    [
        ("start", menu_start(l)),
        ("balance", menu_balance(l)),
        ("bets", menu_bets(l)),
        ("history", menu_history(l)),
        ("send", menu_send(l)),
        ("predict", menu_predict(l)),
        ("events", menu_markets(l)),
        ("rule", menu_rule(l)),
        ("feedback", menu_feedback(l)),
        ("settings", menu_settings(l)),
    ]
}

fn menu_markets(l: Lang) -> &'static str {
    tr!(l;
        "Browse markets", "瀏覽市場", "浏览市场", "マーケットを見る", "마켓 둘러보기",
        "Обзор рынков", "Parcourir les marchés", "Ver mercados", "Märkte ansehen", "Xem thị trường",
        "Lihat pasar", "Tingnan ang mga market", "ดูตลาด", "Markten bekijken", "Piyasalara göz at",
        "Ver mercados", "मार्केट देखें", "تصفّح الأسواق")
}

fn menu_feedback(l: Lang) -> &'static str {
    tr!(l;
        "Send feedback", "意見回饋", "意见反馈", "フィードバック", "피드백 보내기",
        "Отправить отзыв", "Envoyer un avis", "Enviar comentarios", "Feedback senden", "Gửi phản hồi",
        "Kirim masukan", "Magpadala ng feedback", "ส่งความคิดเห็น", "Feedback sturen", "Geri bildirim gönder",
        "Enviar feedback", "फ़ीडबैक भेजें", "إرسال ملاحظات")
}

pub fn feedback_ask(l: Lang) -> &'static str {
    tr!(l;
        "💬 What's your feedback? Type your message and I'll pass it to the team.", "💬 你想回饋什麼？直接打字，我會轉交給團隊。", "💬 你想反馈什么？直接打字，我会转交给团队。", "💬 フィードバックは何かな？そのまま入力してくれれば、チームに伝えるよ。", "💬 어떤 의견이야? 그냥 입력하면 팀에 전달할게.",
        "💬 Какой у вас отзыв? Просто напишите, и я передам команде.", "💬 Quel est ton retour ? Écris-le et je le transmets à l'équipe.", "💬 ¿Cuál es tu comentario? Escríbelo y lo paso al equipo.", "💬 Was ist dein Feedback? Schreib es einfach und ich leite es ans Team weiter.", "💬 Bạn muốn góp ý gì? Cứ nhập vào, mình sẽ chuyển cho đội ngũ.",
        "💬 Apa masukanmu? Ketik saja, akan kuteruskan ke tim.", "💬 Ano ang feedback mo? I-type mo lang at ipapasa ko sa team.", "💬 อยากบอกอะไรเรา? พิมพ์มาได้เลย เดี๋ยวส่งให้ทีมงาน", "💬 Wat is je feedback? Typ het gewoon en ik geef het door aan het team.", "💬 Geri bildirimin ne? Yaz yeter, ekibe iletirim.",
        "💬 Qual é o seu feedback? Escreva e eu repasso para a equipe.", "💬 आपका फ़ीडबैक क्या है? बस टाइप करें, मैं टीम तक पहुँचा दूँगा।", "💬 ما هي ملاحظتك؟ اكتبها وسأنقلها إلى الفريق.")
}

pub fn feedback_check_dm(l: Lang) -> &'static str {
    tr!(l;
        "Check your DM to send feedback 📩", "請查看私訊來傳送回饋 📩", "请查看私信来发送反馈 📩", "DMを確認してフィードバックを送ってね 📩", "DM에서 의견을 보내줘 📩",
        "Проверьте личные сообщения, чтобы отправить отзыв 📩", "Va en privé pour envoyer ton retour 📩", "Revisa tu DM para enviar tu comentario 📩", "Schau in deine DMs, um Feedback zu senden 📩", "Kiểm tra tin nhắn riêng để gửi góp ý 📩",
        "Cek DM untuk mengirim masukan 📩", "Tingnan ang DM mo para magpadala ng feedback 📩", "เช็ค DM เพื่อส่งความคิดเห็น 📩", "Check je DM om feedback te sturen 📩", "Geri bildirim göndermek için DM'ine bak 📩",
        "Veja sua DM para enviar feedback 📩", "फ़ीडबैक भेजने के लिए अपना DM देखें 📩", "تحقق من رسائلك الخاصة لإرسال ملاحظاتك 📩")
}

pub fn feedback_dm_first(l: Lang) -> &'static str {
    tr!(l;
        "Start a private chat with me first to send feedback 📩", "請先私訊我才能傳送回饋 📩", "请先私信我才能发送反馈 📩", "フィードバックを送るにはまず私にDMしてね 📩", "의견을 보내려면 먼저 나에게 DM을 보내줘 📩",
        "Сначала напишите мне в личку, чтобы отправить отзыв 📩", "Écris-moi d'abord en privé pour envoyer ton retour 📩", "Primero abre un chat privado conmigo para enviar tu comentario 📩", "Schreib mir zuerst privat, um Feedback zu senden 📩", "Hãy nhắn riêng cho mình trước để gửi góp ý 📩",
        "Mulai chat pribadi dulu untuk mengirim masukan 📩", "Mag-DM muna sa akin para magpadala ng feedback 📩", "เริ่มแชทส่วนตัวกับฉันก่อนเพื่อส่งความคิดเห็น 📩", "Begin eerst een privégesprek met mij om feedback te sturen 📩", "Geri bildirim göndermek için önce bana özelden yaz 📩",
        "Abra um chat privado comigo primeiro para enviar feedback 📩", "फ़ीडबैक भेजने के लिए पहले मुझसे निजी चैट शुरू करें 📩", "ابدأ محادثة خاصة معي أولًا لإرسال ملاحظاتك 📩")
}

pub fn feedback_sent(l: Lang) -> &'static str {
    tr!(l;
        "Thanks! Your feedback was sent 🙏", "感謝！你的意見已送出 🙏", "感谢！你的反馈已发送 🙏", "ありがとう！フィードバックを送信したよ 🙏", "고마워요! 피드백을 보냈어요 🙏",
        "Спасибо! Ваш отзыв отправлен 🙏", "Merci ! Ton avis a été envoyé 🙏", "¡Gracias! Tu comentario fue enviado 🙏", "Danke! Dein Feedback wurde gesendet 🙏", "Cảm ơn! Phản hồi của bạn đã được gửi 🙏",
        "Terima kasih! Masukanmu telah dikirim 🙏", "Salamat! Naipadala na ang feedback mo 🙏", "ขอบคุณ! ส่งความคิดเห็นของคุณแล้ว 🙏", "Bedankt! Je feedback is verzonden 🙏", "Teşekkürler! Geri bildirimin gönderildi 🙏",
        "Obrigado! Seu feedback foi enviado 🙏", "धन्यवाद! आपका फ़ीडबैक भेज दिया गया 🙏", "شكرًا! تم إرسال ملاحظاتك 🙏")
}

fn menu_rule(l: Lang) -> &'static str {
    tr!(l;
        "How to earn coins", "如何賺幣", "如何赚币", "コインの稼ぎ方", "코인 버는 법",
        "Как заработать монеты", "Comment gagner des pièces", "Cómo ganar monedas", "Münzen verdienen", "Cách kiếm xu",
        "Cara mendapat koin", "Paano kumita ng coins", "วิธีหาเหรียญ", "Munten verdienen", "Para nasıl kazanılır",
        "Como ganhar moedas", "कॉइन कैसे कमाएँ", "كيفية كسب العملات")
}

/// Body of `/rule`: how a user earns coins. `{checkin}` and `{referral}` are the
/// daily-reward and per-invite reward amounts (already formatted whole coins).
pub fn rules_text(l: Lang, checkin: &str, referral: &str) -> String {
    tr!(l;
        "📜 How to earn coins:\n\n🎁 Daily check-in — {checkin} coins every day (resets at 00:00 UTC)\n🔗 Invite a friend — {referral} coins each for you and them\n➕ Add me to a group — new members who first use me there become your referrals too\n👑 Group owner split — the group owner and the inviter share each referral reward 50/50\n👥 Referral bonus — when your invitees check in daily, you earn 1 / 0.1 / 0.01 coins up to 3 levels up\n🎲 Win bets & predictions — payouts land straight in your balance",
        "📜 如何賺幣：\n\n🎁 每日簽到 — 每天 {checkin} 顆金幣（00:00 UTC 重置）\n🔗 邀請好友 — 你和好友各得 {referral} 顆金幣\n➕ 把我加進群組 — 新成員首次在群裡使用我，同樣成為你的推薦\n👑 群主分潤 — 群主與邀請者平分每筆推薦獎勵\n👥 推薦獎勵 — 你邀請的人每日簽到時，你向上最多 3 層各得 1 / 0.1 / 0.01 顆金幣\n🎲 贏得下注與預測 — 彩金直接入帳",
        "📜 如何赚金币：\n\n🎁 每日签到 — 每天 {checkin} 颗金币（00:00 UTC 重置）\n🔗 邀请好友 — 你和好友各得 {referral} 颗金币\n➕ 把我加进群组 — 新成员首次在群里使用我，同样成为你的推荐\n👑 群主分润 — 群主与邀请者平分每笔推荐奖励\n👥 推荐奖励 — 你邀请的人每日签到时，你向上最多 3 层各得 1 / 0.1 / 0.01 颗金币\n🎲 赢得下注与预测 — 彩金直接入账",
        "📜 コインの稼ぎ方：\n\n🎁 デイリーチェックイン — 毎日 {checkin} コイン（00:00 UTC にリセット）\n🔗 友達を招待 — あなたと友達に各 {referral} コイン\n➕ グループに追加 — 新メンバーがそこで初めて私を使うと、同じくあなたの紹介になるよ\n👑 グループ主の分配 — グループ作成者と招待者が紹介報酬を 50/50 で分け合う\n👥 紹介ボーナス — 招待した人が毎日チェックインすると、最大 3 階層上まで 1 / 0.1 / 0.01 コイン\n🎲 ベットと予測に勝つ — 配当は残高に直接入るよ",
        "📜 코인 버는 법:\n\n🎁 일일 출석 — 매일 {checkin} 코인 (00:00 UTC 초기화)\n🔗 친구 초대 — 너와 친구가 각각 {referral} 코인\n➕ 그룹에 추가 — 새 멤버가 거기서 나를 처음 쓰면 똑같이 네 추천이 돼\n👑 그룹장 분배 — 그룹장과 초대자가 추천 보상을 50/50으로 나눠 가져\n👥 추천 보너스 — 초대한 사람이 매일 출석하면 위로 최대 3단계까지 1 / 0.1 / 0.01 코인\n🎲 베팅·예측 승리 — 당첨금은 잔액에 바로 들어와",
        "📜 Как заработать монеты:\n\n🎁 Ежедневный чек-ин — {checkin} монет каждый день (сброс в 00:00 UTC)\n🔗 Пригласи друга — по {referral} монет тебе и ему\n➕ Добавь меня в группу — новые участники, впервые использующие меня там, тоже становятся твоими рефералами\n👑 Доля владельца группы — владелец группы и пригласивший делят каждую реферальную награду 50/50\n👥 Реферальный бонус — когда твои приглашённые отмечаются, ты получаешь 1 / 0.1 / 0.01 монеты до 3 уровней вверх\n🎲 Выигрывай ставки и прогнозы — выплаты сразу на баланс",
        "📜 Comment gagner des pièces :\n\n🎁 Check-in quotidien — {checkin} pièces chaque jour (remise à 00:00 UTC)\n🔗 Invite un ami — {referral} pièces chacun pour toi et lui\n➕ Ajoute-moi à un groupe — les nouveaux membres qui m'y utilisent en premier deviennent aussi tes filleuls\n👑 Part du propriétaire du groupe — le propriétaire du groupe et l'inviteur partagent chaque récompense de parrainage 50/50\n👥 Bonus de parrainage — quand tes invités font leur check-in, tu gagnes 1 / 0,1 / 0,01 pièces jusqu'à 3 niveaux au-dessus\n🎲 Gagne paris et prédictions — les gains arrivent direct sur ton solde",
        "📜 Cómo ganar monedas:\n\n🎁 Check-in diario — {checkin} monedas cada día (se reinicia a las 00:00 UTC)\n🔗 Invita a un amigo — {referral} monedas para cada uno\n➕ Añádeme a un grupo — los nuevos miembros que me usen ahí por primera vez también se vuelven tus referidos\n👑 Parte del dueño del grupo — el dueño del grupo y quien invita se reparten cada recompensa de referido 50/50\n👥 Bono de referidos — cuando tus invitados hacen check-in, ganas 1 / 0,1 / 0,01 monedas hasta 3 niveles arriba\n🎲 Gana apuestas y predicciones — los pagos van directo a tu saldo",
        "📜 Münzen verdienen:\n\n🎁 Täglicher Check-in — {checkin} Münzen pro Tag (Reset um 00:00 UTC)\n🔗 Lade einen Freund ein — je {referral} Münzen für dich und ihn\n➕ Füge mich zu einer Gruppe hinzu — neue Mitglieder, die mich dort zuerst nutzen, werden auch deine Empfohlenen\n👑 Gruppenbesitzer-Anteil — Gruppenbesitzer und Einlader teilen sich jede Empfehlungsprämie 50/50\n👥 Empfehlungsbonus — wenn deine Eingeladenen einchecken, bekommst du 1 / 0,1 / 0,01 Münzen bis zu 3 Ebenen höher\n🎲 Gewinne Wetten & Vorhersagen — Auszahlungen landen direkt auf deinem Guthaben",
        "📜 Cách kiếm xu:\n\n🎁 Điểm danh hằng ngày — {checkin} xu mỗi ngày (đặt lại lúc 00:00 UTC)\n🔗 Mời bạn bè — bạn và bạn của bạn mỗi người {referral} xu\n➕ Thêm mình vào nhóm — thành viên mới lần đầu dùng mình ở đó cũng trở thành người bạn giới thiệu\n👑 Chia cho chủ nhóm — chủ nhóm và người mời chia đôi mỗi phần thưởng giới thiệu 50/50\n👥 Thưởng giới thiệu — khi người bạn mời điểm danh, bạn nhận 1 / 0.1 / 0.01 xu lên tới 3 cấp\n🎲 Thắng cược & dự đoán — tiền thắng vào thẳng số dư",
        "📜 Cara mendapat koin:\n\n🎁 Check-in harian — {checkin} koin tiap hari (reset pukul 00:00 UTC)\n🔗 Undang teman — {referral} koin untuk kamu dan dia\n➕ Tambahkan aku ke grup — anggota baru yang pertama memakaiku di sana juga jadi referralmu\n👑 Bagi hasil pemilik grup — pemilik grup dan pengundang berbagi tiap hadiah referral 50/50\n👥 Bonus referral — saat undanganmu check-in, kamu dapat 1 / 0.1 / 0.01 koin hingga 3 tingkat ke atas\n🎲 Menang taruhan & prediksi — hadiah langsung masuk saldo",
        "📜 Paano kumita ng coins:\n\n🎁 Daily check-in — {checkin} coins kada araw (nire-reset tuwing 00:00 UTC)\n🔗 Mag-imbita ng kaibigan — {referral} coins kayo pareho\n➕ Idagdag mo ako sa grupo — ang bagong members na unang gumamit sa akin doon ay magiging referral mo rin\n👑 Hati ng may-ari ng grupo — hinahati ng may-ari ng grupo at ng nag-imbita ang bawat referral reward 50/50\n👥 Referral bonus — kapag nag-check-in ang inimbita mo, kumikita ka ng 1 / 0.1 / 0.01 coins hanggang 3 antas pataas\n🎲 Manalo sa taya at prediksyon — diretso sa balance ang panalo",
        "📜 วิธีหาเหรียญ:\n\n🎁 เช็คอินรายวัน — {checkin} เหรียญทุกวัน (รีเซ็ต 00:00 UTC)\n🔗 ชวนเพื่อน — คุณและเพื่อนได้คนละ {referral} เหรียญ\n➕ เพิ่มฉันเข้ากลุ่ม — สมาชิกใหม่ที่เริ่มใช้ฉันที่นั่นก็จะกลายเป็นคนที่คุณแนะนำเช่นกัน\n👑 ส่วนแบ่งเจ้าของกลุ่ม — เจ้าของกลุ่มและผู้เชิญแบ่งรางวัลแนะนำคนละครึ่ง\n👥 โบนัสแนะนำ — เมื่อคนที่คุณชวนเช็คอิน คุณได้ 1 / 0.1 / 0.01 เหรียญ สูงสุด 3 ชั้น\n🎲 ชนะเดิมพันและการทำนาย — เงินรางวัลเข้ายอดทันที",
        "📜 Munten verdienen:\n\n🎁 Dagelijkse check-in — {checkin} munten per dag (reset om 00:00 UTC)\n🔗 Nodig een vriend uit — {referral} munten voor jullie allebei\n➕ Voeg me toe aan een groep — nieuwe leden die me daar voor het eerst gebruiken worden ook jouw referrals\n👑 Aandeel groepseigenaar — groepseigenaar en uitnodiger delen elke verwijzingsbeloning 50/50\n👥 Verwijzingsbonus — als je uitgenodigden inchecken, verdien je 1 / 0,1 / 0,01 munten tot 3 niveaus hoger\n🎲 Win weddenschappen & voorspellingen — uitbetalingen komen direct op je saldo",
        "📜 Para nasıl kazanılır:\n\n🎁 Günlük giriş — her gün {checkin} para (00:00 UTC'de sıfırlanır)\n🔗 Bir arkadaşını davet et — ikinize de {referral} para\n➕ Beni bir gruba ekle — orada beni ilk kez kullanan yeni üyeler de senin davetlin olur\n👑 Grup sahibi payı — grup sahibi ve davet eden her davet ödülünü 50/50 paylaşır\n👥 Davet bonusu — davet ettiklerin giriş yaptığında 3 seviye yukarıya kadar 1 / 0,1 / 0,01 para kazanırsın\n🎲 Bahis ve tahmin kazan — ödemeler doğrudan bakiyene geçer",
        "📜 Como ganhar moedas:\n\n🎁 Check-in diário — {checkin} moedas por dia (reinicia às 00:00 UTC)\n🔗 Convide um amigo — {referral} moedas para cada um\n➕ Adicione-me a um grupo — novos membros que me usarem lá pela primeira vez também viram seus indicados\n👑 Parte do dono do grupo — o dono do grupo e quem convida dividem cada recompensa de indicação 50/50\n👥 Bônus de indicação — quando seus convidados fazem check-in, você ganha 1 / 0,1 / 0,01 moedas até 3 níveis acima\n🎲 Vença apostas e previsões — os prêmios caem direto no saldo",
        "📜 कॉइन कैसे कमाएँ:\n\n🎁 दैनिक चेक-इन — हर दिन {checkin} कॉइन (00:00 UTC पर रीसेट)\n🔗 दोस्त को बुलाएँ — आप और दोस्त दोनों को {referral} कॉइन\n➕ मुझे ग्रुप में जोड़ें — वहाँ पहली बार मुझे इस्तेमाल करने वाले नए सदस्य भी आपके रेफ़रल बन जाते हैं\n👑 ग्रुप मालिक हिस्सा — ग्रुप मालिक और आमंत्रक हर रेफ़रल इनाम 50/50 बाँटते हैं\n👥 रेफ़रल बोनस — आपके बुलाए लोग चेक-इन करें तो आपको 3 स्तर ऊपर तक 1 / 0.1 / 0.01 कॉइन\n🎲 बेट और प्रिडिक्शन जीतें — जीत सीधे बैलेंस में",
        "📜 كيفية كسب العملات:\n\n🎁 تسجيل يومي — {checkin} عملة كل يوم (يُعاد ضبطه عند 00:00 UTC)\n🔗 ادعُ صديقًا — {referral} عملة لكل منكما\n➕ أضِفني إلى مجموعة — الأعضاء الجدد الذين يستخدمونني هناك لأول مرة يصبحون أيضًا من إحالاتك\n👑 حصة مالك المجموعة — يتقاسم مالك المجموعة والداعي كل مكافأة إحالة مناصفة\n👥 مكافأة الإحالة — عندما يسجّل من دعوتهم، تكسب 1 / 0.1 / 0.01 عملة حتى 3 مستويات للأعلى\n🎲 اربح الرهانات والتوقعات — تذهب الأرباح مباشرة إلى رصيدك")
    .replace("{checkin}", checkin)
    .replace("{referral}", referral)
}

fn menu_start(l: Lang) -> &'static str {
    tr!(l;
        "Home page", "首頁", "首页", "ホーム", "홈",
        "Главная", "Accueil", "Inicio", "Startseite", "Trang chính",
        "Beranda", "Home", "หน้าหลัก", "Startpagina", "Ana sayfa",
        "Página inicial", "होम", "الصفحة الرئيسية")
}

/// `🏠 Home page` button label — sends a terminal flow node (a placed bet, a
/// funded stake, an expired board, a created prediction) back to the `/start`
/// menu instead of dead-ending.
pub fn home_btn(l: Lang) -> String {
    format!("🏠 {}", menu_start(l))
}

fn menu_balance(l: Lang) -> &'static str {
    tr!(l;
        "Check balance", "查看餘額", "查看余额", "残高を確認", "잔액 확인",
        "Проверить баланс", "Voir le solde", "Ver saldo", "Guthaben prüfen", "Xem số dư",
        "Cek saldo", "Tingnan ang balanse", "ดูยอดเงิน", "Saldo bekijken", "Bakiyeyi gör",
        "Ver saldo", "बैलेंस देखें", "عرض الرصيد")
}

fn menu_bets(l: Lang) -> &'static str {
    tr!(l;
        "Open bets", "未結算下注", "未结算下注", "未決済のベット", "미정산 베팅",
        "Открытые ставки", "Paris en cours", "Apuestas abiertas", "Offene Wetten", "Cược đang mở",
        "Taruhan terbuka", "Bukas na taya", "เดิมพันที่เปิดอยู่", "Open weddenschappen", "Açık bahisler",
        "Apostas abertas", "खुले दांव", "الرهانات المفتوحة")
}

fn menu_settings(l: Lang) -> &'static str {
    tr!(l;
        "Preferences", "偏好設定", "偏好设置", "設定", "환경설정",
        "Настройки", "Préférences", "Preferencias", "Einstellungen", "Tùy chỉnh",
        "Preferensi", "Mga kagustuhan", "การตั้งค่า", "Voorkeuren", "Tercihler",
        "Preferências", "सेटिंग्स", "التفضيلات")
}

/// `/settings` hub header. The three buttons (Language / Timezone / Format)
/// each open a picker that ✅-marks the current choice.
pub fn settings_title(l: Lang) -> &'static str {
    tr!(l;
        "⚙️ Settings", "⚙️ 設定", "⚙️ 设置", "⚙️ 設定", "⚙️ 설정",
        "⚙️ Настройки", "⚙️ Paramètres", "⚙️ Ajustes", "⚙️ Einstellungen", "⚙️ Cài đặt",
        "⚙️ Pengaturan", "⚙️ Mga Setting", "⚙️ การตั้งค่า", "⚙️ Instellingen", "⚙️ Ayarlar",
        "⚙️ Configurações", "⚙️ सेटिंग्स", "⚙️ الإعدادات")
}

/// In a group, `/settings` opens privately — point the user to their DM.
pub fn settings_check_dm(l: Lang) -> &'static str {
    tr!(l;
        "Check your DM to change settings ⚙️📩", "請查看私訊來變更設定 ⚙️📩", "请查看私信来更改设置 ⚙️📩", "設定はDMで変更してね ⚙️📩", "설정은 DM에서 바꿔줘 ⚙️📩",
        "Откройте личные сообщения, чтобы изменить настройки ⚙️📩", "Va en privé pour changer les paramètres ⚙️📩", "Revisa tu DM para cambiar los ajustes ⚙️📩", "Schau in deine DMs, um Einstellungen zu ändern ⚙️📩", "Kiểm tra tin nhắn riêng để đổi cài đặt ⚙️📩",
        "Cek DM untuk mengubah pengaturan ⚙️📩", "Tingnan ang DM mo para baguhin ang settings ⚙️📩", "เช็ค DM เพื่อเปลี่ยนการตั้งค่า ⚙️📩", "Check je DM om instellingen te wijzigen ⚙️📩", "Ayarları değiştirmek için DM'ine bak ⚙️📩",
        "Veja sua DM para mudar as configurações ⚙️📩", "सेटिंग्स बदलने के लिए अपना DM देखें ⚙️📩", "تحقق من رسائلك الخاصة لتغيير الإعدادات ⚙️📩")
}

/// `/settings` in a group but the user never opened a DM with the bot.
pub fn settings_dm_first(l: Lang) -> &'static str {
    tr!(l;
        "Start a private chat with me first to change settings 📩", "請先私訊我才能變更設定 📩", "请先私信我才能更改设置 📩", "設定を変えるにはまず私にDMしてね 📩", "설정을 바꾸려면 먼저 나에게 DM을 보내줘 📩",
        "Сначала напишите мне в личку, чтобы менять настройки 📩", "Écris-moi d'abord en privé pour changer les paramètres 📩", "Primero abre un chat privado conmigo para cambiar los ajustes 📩", "Schreib mir zuerst privat, um Einstellungen zu ändern 📩", "Hãy nhắn riêng cho mình trước để đổi cài đặt 📩",
        "Mulai chat pribadi dulu untuk mengubah pengaturan 📩", "Mag-DM muna sa akin para baguhin ang settings 📩", "เริ่มแชทส่วนตัวกับฉันก่อนเพื่อเปลี่ยนการตั้งค่า 📩", "Begin eerst een privégesprek met mij om instellingen te wijzigen 📩", "Ayarları değiştirmek için önce bana özelden yaz 📩",
        "Abra um chat privado comigo primeiro para mudar as configurações 📩", "सेटिंग्स बदलने के लिए पहले मुझसे निजी चैट शुरू करें 📩", "ابدأ محادثة خاصة معي أولًا لتغيير الإعدادات 📩")
}

/// `[🌐 Language]` button in the `/settings` hub (opens the language picker).
pub fn btn_language(l: Lang) -> &'static str {
    tr!(l;
        "🌐 Language", "🌐 語言", "🌐 语言", "🌐 言語", "🌐 언어",
        "🌐 Язык", "🌐 Langue", "🌐 Idioma", "🌐 Sprache", "🌐 Ngôn ngữ",
        "🌐 Bahasa", "🌐 Wika", "🌐 ภาษา", "🌐 Taal", "🌐 Dil",
        "🌐 Idioma", "🌐 भाषा", "🌐 اللغة")
}

/// `[🕐 Timezone]` button in the `/settings` hub (opens the timezone picker).
pub fn btn_timezone(l: Lang) -> &'static str {
    tr!(l;
        "🕐 Timezone", "🕐 時區", "🕐 时区", "🕐 タイムゾーン", "🕐 시간대",
        "🕐 Часовой пояс", "🕐 Fuseau horaire", "🕐 Zona horaria", "🕐 Zeitzone", "🕐 Múi giờ",
        "🕐 Zona waktu", "🕐 Time zone", "🕐 โซนเวลา", "🕐 Tijdzone", "🕐 Saat dilimi",
        "🕐 Fuso horário", "🕐 टाइमज़ोन", "🕐 المنطقة الزمنية")
}

/// `[🎲 Format]` button in the `/settings` hub (opens the odds-format picker).
pub fn btn_odds(l: Lang) -> &'static str {
    tr!(l;
        "🎲 Format", "🎲 格式", "🎲 格式", "🎲 形式", "🎲 형식",
        "🎲 Формат", "🎲 Format", "🎲 Formato", "🎲 Format", "🎲 Định dạng",
        "🎲 Format", "🎲 Format", "🎲 รูปแบบ", "🎲 Formaat", "🎲 Biçim",
        "🎲 Formato", "🎲 फ़ॉर्मैट", "🎲 الصيغة")
}

fn menu_send(l: Lang) -> &'static str {
    tr!(l;
        "Transfer assets", "轉移資產", "转移资产", "資産を送る", "자산 이전",
        "Перевести активы", "Transférer des actifs", "Transferir activos", "Vermögen übertragen", "Chuyển tài sản",
        "Transfer aset", "Maglipat ng asset", "โอนสินทรัพย์", "Activa overdragen", "Varlık aktar",
        "Transferir ativos", "संपत्ति ट्रांसफर करें", "تحويل الأصول")
}

fn menu_predict(l: Lang) -> &'static str {
    tr!(l;
        "Create a prediction", "開啟預測", "开启预测", "予測を開く", "예측 열기",
        "Открыть прогноз", "Ouvrir une prédiction", "Abrir predicción", "Vorhersage öffnen", "Mở dự đoán",
        "Buka prediksi", "Magbukas ng prediksyon", "เปิดการทำนาย", "Voorspelling openen", "Tahmin aç",
        "Abrir previsão", "प्रिडिक्शन खोलें", "افتح التوقع")
}

/// Per-staker settlement line, e.g. `***1234 won 50 coins`. `verb` is
/// already localized via [`verb_won`] / [`verb_lost`].
pub fn settle_line(l: Lang, name: &str, verb: &str, amt: &str) -> String {
    tr!(l;
        "\n{name} {verb} {amt} coins", "\n{name} {verb}{amt}顆 金幣", "\n{name} {verb}{amt}颗 金币", "\n{name} コイン{amt}枚{verb}", "\n{name} 코인 {amt}개 {verb}",
        "\n{name} {verb} {amt} монет", "\n{name} {verb} {amt} pièces", "\n{name} {verb} {amt} monedas", "\n{name} {verb} {amt} Münzen", "\n{name} {verb} {amt} xu",
        "\n{name} {verb} {amt} koin", "\n{name} {verb} {amt} coins", "\n{name} {verb} {amt} เหรียญ", "\n{name} {verb} {amt} munten", "\n{name} {amt} para {verb}",
        "\n{name} {verb} {amt} moedas", "\n{name} {amt} कॉइन {verb}", "\n{name} {verb} {amt} عملة")
    .replace("{name}", name)
    .replace("{verb}", verb)
    .replace("{amt}", amt)
}

/// Tail line when the settlement readout is capped to the top winners.
pub fn more_winners(l: Lang, n: &str) -> String {
    tr!(l;
        "\n…and {n} more winners", "\n…還有 {n} 位贏家", "\n…还有 {n} 位赢家", "\n…ほか {n} 人の勝者", "\n…외 {n}명의 승자",
        "\n…и ещё {n} победителей", "\n…et {n} autres gagnants", "\n…y {n} ganadores más", "\n…und {n} weitere Gewinner", "\n…và {n} người thắng nữa",
        "\n…dan {n} pemenang lagi", "\n…at {n} pang nanalo", "\n…และผู้ชนะอีก {n} คน", "\n…en nog {n} winnaars", "\n…ve {n} kazanan daha",
        "\n…e mais {n} vencedores", "\n…और {n} विजेता", "\n…و{n} فائزين آخرين")
    .replace("{n}", n)
}

// ----------------------------------------------------------------------------
// Command-usage hints (shown when a command is called with the wrong format)
// ----------------------------------------------------------------------------

pub fn usage_send(l: Lang) -> &'static str {
    tr!(l;
        "Reply to a message, then /send <amount>", "回覆一則訊息，然後 /send <數量>", "回复一条消息，然后 /send <数量>", "メッセージに返信して /send <数量>", "메시지에 답장하고 /send <수량>",
        "Ответьте на сообщение, затем /send <сумма>", "Réponds à un message, puis /send <montant>", "Responde a un mensaje y usa /send <cantidad>", "Antworte auf eine Nachricht, dann /send <Betrag>", "Trả lời một tin nhắn rồi /send <số lượng>",
        "Balas pesan, lalu /send <jumlah>", "Mag-reply sa mensahe, tapos /send <halaga>", "ตอบกลับข้อความ แล้ว /send <จำนวน>", "Reageer op een bericht, dan /send <bedrag>", "Bir mesajı yanıtla, sonra /send <miktar>",
        "Responda a uma mensagem e use /send <quantia>", "किसी संदेश का जवाब दें, फिर /send <राशि>", "ردّ على رسالة ثم /send <المبلغ>")
}

pub fn usage_sell(l: Lang) -> &'static str {
    tr!(l;
        "/sell <fruit> <price>", "/sell <水果> <價格>", "/sell <水果> <价格>", "/sell <フルーツ> <価格>", "/sell <과일> <가격>",
        "/sell <фрукт> <цена>", "/sell <fruit> <prix>", "/sell <fruta> <precio>", "/sell <Obst> <Preis>", "/sell <trái cây> <giá>",
        "/sell <buah> <harga>", "/sell <prutas> <presyo>", "/sell <ผลไม้> <ราคา>", "/sell <fruit> <prijs>", "/sell <meyve> <fiyat>",
        "/sell <fruta> <preço>", "/sell <फल> <मूल्य>", "/sell <فاكهة> <سعر>")
}

pub fn usage_buy(l: Lang) -> &'static str {
    tr!(l;
        "/buy <fruit> <price>", "/buy <水果> <價格>", "/buy <水果> <价格>", "/buy <フルーツ> <価格>", "/buy <과일> <가격>",
        "/buy <фрукт> <цена>", "/buy <fruit> <prix>", "/buy <fruta> <precio>", "/buy <Obst> <Preis>", "/buy <trái cây> <giá>",
        "/buy <buah> <harga>", "/buy <prutas> <presyo>", "/buy <ผลไม้> <ราคา>", "/buy <fruit> <prijs>", "/buy <meyve> <fiyat>",
        "/buy <fruta> <preço>", "/buy <फल> <मूल्य>", "/buy <فاكهة> <سعر>")
}

// --- /predict DM builder wizard ---

pub fn predict_ask_question(l: Lang) -> &'static str {
    tr!(l;
        "🎲 What's your prediction question?", "🎲 你的預測問題是什麼？", "🎲 你的预测问题是什么？", "🎲 予測する質問は？", "🎲 예측 질문은 무엇이야?",
        "🎲 Какой у вас вопрос для прогноза?", "🎲 Quelle est ta question de prédiction ?", "🎲 ¿Cuál es tu pregunta de predicción?", "🎲 Wie lautet deine Vorhersagefrage?", "🎲 Câu hỏi dự đoán của bạn là gì?",
        "🎲 Apa pertanyaan prediksimu?", "🎲 Ano ang tanong ng prediksyon mo?", "🎲 คำถามทำนายของคุณคืออะไร?", "🎲 Wat is je voorspellingsvraag?", "🎲 Tahmin sorun nedir?",
        "🎲 Qual é a sua pergunta de previsão?", "🎲 आपका प्रिडिक्शन सवाल क्या है?", "🎲 ما هو سؤال توقعك؟")
}

pub fn predict_ask_options(l: Lang) -> &'static str {
    tr!(l;
        "Now send the options — one per line, or space-separated (at least 2).", "現在傳送選項 — 每行一個，或以空格分隔（至少 2 個）。", "现在发送选项 — 每行一个，或用空格分隔（至少 2 个）。", "選択肢を送ってね — 1行に1つ、またはスペース区切り（2つ以上）。", "이제 선택지를 보내줘 — 한 줄에 하나, 또는 공백으로 구분 (최소 2개).",
        "Теперь пришлите варианты — по одному на строку или через пробел (минимум 2).", "Envoie maintenant les options — une par ligne, ou séparées par des espaces (au moins 2).", "Ahora envía las opciones — una por línea, o separadas por espacios (al menos 2).", "Sende jetzt die Optionen — eine pro Zeile oder durch Leerzeichen getrennt (mindestens 2).", "Bây giờ gửi các lựa chọn — mỗi dòng một cái, hoặc cách nhau bằng dấu cách (ít nhất 2).",
        "Sekarang kirim opsinya — satu per baris, atau dipisah spasi (minimal 2).", "Ipadala na ang mga opsyon — isa kada linya, o pinaghihiwalay ng space (hindi bababa sa 2).", "ตอนนี้ส่งตัวเลือก — บรรทัดละหนึ่ง หรือคั่นด้วยช่องว่าง (อย่างน้อย 2)", "Stuur nu de opties — één per regel, of gescheiden door spaties (minstens 2).", "Şimdi seçenekleri gönder — her satıra bir tane veya boşlukla ayrılmış (en az 2).",
        "Agora envie as opções — uma por linha, ou separadas por espaços (pelo menos 2).", "अब विकल्प भेजें — हर लाइन में एक, या स्पेस से अलग (कम से कम 2)।", "أرسل الآن الخيارات — واحد في كل سطر، أو مفصولة بمسافات (على الأقل 2).")
}

pub fn predict_ask_endtime(l: Lang) -> &'static str {
    tr!(l;
        "When does betting close?", "下注何時截止？", "下注何时截止？", "賭けはいつ締め切る？", "베팅은 언제 마감할까?",
        "Когда закрыть приём ставок?", "Quand les paris ferment-ils ?", "¿Cuándo cierran las apuestas?", "Wann schließen die Wetten?", "Khi nào đóng cược?",
        "Kapan taruhan ditutup?", "Kailan magsasara ang taya?", "ปิดรับเดิมพันเมื่อไหร่?", "Wanneer sluit het wedden?", "Bahisler ne zaman kapansın?",
        "Quando as apostas fecham?", "बेटिंग कब बंद होगी?", "متى يُغلق الرهان؟")
}

pub fn predict_need_options(l: Lang) -> &'static str {
    tr!(l;
        "Send at least 2 options 🙏", "請至少傳送 2 個選項 🙏", "请至少发送 2 个选项 🙏", "選択肢は2つ以上送ってね 🙏", "선택지를 2개 이상 보내줘 🙏",
        "Пришлите минимум 2 варианта 🙏", "Envoie au moins 2 options 🙏", "Envía al menos 2 opciones 🙏", "Sende mindestens 2 Optionen 🙏", "Gửi ít nhất 2 lựa chọn 🙏",
        "Kirim minimal 2 opsi 🙏", "Magpadala ng hindi bababa sa 2 opsyon 🙏", "ส่งตัวเลือกอย่างน้อย 2 อัน 🙏", "Stuur minstens 2 opties 🙏", "En az 2 seçenek gönder 🙏",
        "Envie pelo menos 2 opções 🙏", "कम से कम 2 विकल्प भेजें 🙏", "أرسل خيارين على الأقل 🙏")
}

pub fn predict_created(l: Lang) -> &'static str {
    tr!(l;
        "✅ Prediction posted!", "✅ 預測已發佈！", "✅ 预测已发布！", "✅ 予測を投稿したよ！", "✅ 예측을 올렸어！",
        "✅ Прогноз опубликован!", "✅ Prédiction publiée !", "✅ ¡Predicción publicada!", "✅ Vorhersage veröffentlicht!", "✅ Đã đăng dự đoán!",
        "✅ Prediksi diposting!", "✅ Nai-post na ang prediksyon!", "✅ โพสต์การทำนายแล้ว!", "✅ Voorspelling geplaatst!", "✅ Tahmin paylaşıldı!",
        "✅ Previsão publicada!", "✅ प्रिडिक्शन पोस्ट हो गया!", "✅ تم نشر التوقع!")
}

pub fn predict_check_dm(l: Lang) -> &'static str {
    tr!(l;
        "Check your DM to build the prediction 📩", "請查看私訊來建立預測 📩", "请查看私信来创建预测 📩", "DMを確認して予測を作ってね 📩", "DM에서 예측을 만들어 📩",
        "Проверьте личные сообщения, чтобы создать прогноз 📩", "Va en privé pour créer la prédiction 📩", "Revisa tu DM para crear la predicción 📩", "Schau in deine DMs, um die Vorhersage zu erstellen 📩", "Kiểm tra tin nhắn riêng để tạo dự đoán 📩",
        "Cek DM untuk membuat prediksi 📩", "Tingnan ang DM mo para gawin ang prediksyon 📩", "เช็ค DM เพื่อสร้างการทำนาย 📩", "Check je DM om de voorspelling te maken 📩", "Tahmini oluşturmak için DM'ine bak 📩",
        "Veja sua DM para criar a previsão 📩", "प्रिडिक्शन बनाने के लिए अपना DM देखें 📩", "تحقق من رسائلك الخاصة لإنشاء التوقع 📩")
}

pub fn predict_post_failed(l: Lang) -> &'static str {
    tr!(l;
        "⚠️ Couldn't post the card — am I still in that chat?", "⚠️ 無法發佈卡片 — 我還在那個對話裡嗎？", "⚠️ 无法发布卡片 — 我还在那个对话里吗？", "⚠️ カードを投稿できなかったよ — まだそのチャットにいる？", "⚠️ 카드를 못 올렸어 — 내가 아직 그 채팅에 있어?",
        "⚠️ Не удалось опубликовать карточку — я ещё в том чате?", "⚠️ Impossible de publier la carte — suis-je encore dans ce chat ?", "⚠️ No pude publicar la tarjeta — ¿sigo en ese chat?", "⚠️ Konnte die Karte nicht posten — bin ich noch in dem Chat?", "⚠️ Không đăng được thẻ — tôi còn trong cuộc trò chuyện đó chứ?",
        "⚠️ Gagal memposting kartu — apakah saya masih di chat itu?", "⚠️ Hindi ma-post ang card — nasa chat na iyon pa ba ako?", "⚠️ โพสต์การ์ดไม่ได้ — ฉันยังอยู่ในแชทนั้นไหม?", "⚠️ Kon de kaart niet plaatsen — zit ik nog in die chat?", "⚠️ Kart paylaşılamadı — hâlâ o sohbette miyim?",
        "⚠️ Não consegui postar o cartão — ainda estou nesse chat?", "⚠️ कार्ड पोस्ट नहीं कर सका — क्या मैं अब भी उस चैट में हूँ?", "⚠️ تعذّر نشر البطاقة — هل ما زلت في تلك المحادثة؟")
}

pub fn btn_custom(l: Lang) -> &'static str {
    tr!(l;
        "⌨️ Custom", "⌨️ 自訂", "⌨️ 自定义", "⌨️ カスタム", "⌨️ 직접 입력",
        "⌨️ Своё", "⌨️ Perso", "⌨️ Personalizado", "⌨️ Eigene", "⌨️ Tùy chỉnh",
        "⌨️ Kustom", "⌨️ Custom", "⌨️ กำหนดเอง", "⌨️ Aangepast", "⌨️ Özel",
        "⌨️ Personalizado", "⌨️ कस्टम", "⌨️ مخصص")
}

/// Prompt after the host taps the custom end-time button: type a duration.
pub fn predict_ask_custom(l: Lang) -> &'static str {
    tr!(l;
        "Type a custom duration — e.g. 2h, 90m, or 1d12h:", "輸入自訂時長 — 例如 2h、90m 或 1d12h：", "输入自定义时长 — 例如 2h、90m 或 1d12h：", "カスタムの長さを入力してね — 例：2h、90m、1d12h：", "직접 기간을 입력해줘 — 예: 2h, 90m, 1d12h:",
        "Введите своё время — напр. 2h, 90m или 1d12h:", "Entre une durée perso — ex. 2h, 90m ou 1d12h :", "Escribe una duración personalizada — ej. 2h, 90m o 1d12h:", "Gib eine eigene Dauer ein — z. B. 2h, 90m oder 1d12h:", "Nhập thời lượng tùy chỉnh — vd. 2h, 90m hoặc 1d12h:",
        "Ketik durasi kustom — mis. 2h, 90m, atau 1d12h:", "Mag-type ng custom na tagal — hal. 2h, 90m, o 1d12h:", "พิมพ์ระยะเวลาที่กำหนดเอง — เช่น 2h, 90m หรือ 1d12h:", "Typ een eigen duur — bijv. 2h, 90m of 1d12h:", "Özel süre yaz — örn. 2h, 90m veya 1d12h:",
        "Digite uma duração personalizada — ex. 2h, 90m ou 1d12h:", "कस्टम अवधि टाइप करें — जैसे 2h, 90m, या 1d12h:", "اكتب مدة مخصصة — مثل 2h أو 90m أو 1d12h:")
}

/// The host's custom duration text didn't parse.
pub fn predict_bad_duration(l: Lang) -> &'static str {
    tr!(l;
        "Couldn't read that — try like 2h, 90m, or 1d12h.", "無法辨識 — 試試 2h、90m 或 1d12h。", "无法识别 — 试试 2h、90m 或 1d12h。", "読み取れなかったよ — 2h、90m、1d12h みたいに入力してね。", "못 알아들었어 — 2h, 90m, 1d12h 처럼 입력해줘.",
        "Не понял — попробуйте как 2h, 90m или 1d12h.", "Format non reconnu — essaie 2h, 90m ou 1d12h.", "No lo entendí — prueba 2h, 90m o 1d12h.", "Nicht erkannt — versuch 2h, 90m oder 1d12h.", "Không đọc được — thử 2h, 90m hoặc 1d12h.",
        "Tidak terbaca — coba 2h, 90m, atau 1d12h.", "Hindi mabasa — subukan ang 2h, 90m, o 1d12h.", "อ่านไม่ออก — ลอง 2h, 90m หรือ 1d12h", "Niet herkend — probeer 2h, 90m of 1d12h.", "Anlaşılmadı — 2h, 90m veya 1d12h dene.",
        "Não entendi — tente 2h, 90m ou 1d12h.", "समझ नहीं आया — 2h, 90m, या 1d12h जैसा लिखें।", "لم أفهم — جرّب 2h أو 90m أو 1d12h.")
}

pub fn btn_no_deadline(l: Lang) -> &'static str {
    tr!(l;
        "♾️ No deadline", "♾️ 無期限", "♾️ 无期限", "♾️ 期限なし", "♾️ 기한 없음",
        "♾️ Без срока", "♾️ Sans limite", "♾️ Sin límite", "♾️ Keine Frist", "♾️ Không giới hạn",
        "♾️ Tanpa batas", "♾️ Walang deadline", "♾️ ไม่มีกำหนด", "♾️ Geen deadline", "♾️ Süresiz",
        "♾️ Sem prazo", "♾️ कोई समय-सीमा नहीं", "♾️ بلا موعد")
}

// ----------------------------------------------------------------------------
// `/start` onboarding & main menu
// ----------------------------------------------------------------------------

/// Neutral, language-agnostic prompt shown before the user has picked a locale.
pub const CHOOSE_LANGUAGE: &str = "🌐 Please choose your language\n请选择语言 · 言語を選択 · 언어 선택";

/// Timezone-picker prompt (shown after the language is chosen, so it's localized).
pub fn choose_timezone(l: Lang) -> &'static str {
    tr!(l;
        "🕒 Pick your timezone:", "🕒 選擇你的時區：", "🕒 选择你的时区：", "🕒 タイムゾーンを選んでね：", "🕒 시간대를 선택하세요:",
        "🕒 Выберите часовой пояс:", "🕒 Choisis ton fuseau horaire :", "🕒 Elige tu zona horaria:", "🕒 Wähle deine Zeitzone:", "🕒 Chọn múi giờ của bạn:",
        "🕒 Pilih zona waktumu:", "🕒 Piliin ang iyong timezone:", "🕒 เลือกเขตเวลาของคุณ:", "🕒 Kies je tijdzone:", "🕒 Saat dilimini seç:",
        "🕒 Escolha seu fuso horário:", "🕒 अपना टाइमज़ोन चुनें:", "🕒 اختر منطقتك الزمنية:")
}

/// Time-of-day bucket for the greeting, from the user's local clock
/// (`util::day_part`). Morning 05:00–11:59, afternoon 12:00–17:59, else evening.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DayPart {
    Morning,
    Afternoon,
    Evening,
}

/// Time-aware salutation for the home menu — prepended to [`intro`], picked from
/// the user's local time (`DayPart`). Region-appropriate per locale (e.g. Taiwan
/// 早安/午安/晚安 vs Mainland 早上好/下午好/晚上好; JA おはようございます/こんにちは/こんばんは).
pub fn greeting(l: Lang, part: DayPart) -> &'static str {
    match part {
        DayPart::Morning => tr!(l;
            "Good morning", "早安", "早上好", "おはようございます", "좋은 아침",
            "Доброе утро", "Bonjour", "Buenos días", "Guten Morgen", "Chào buổi sáng",
            "Selamat pagi", "Magandang umaga", "อรุณสวัสดิ์", "Goedemorgen", "Günaydın",
            "Bom dia", "सुप्रभात", "صباح الخير"),
        DayPart::Afternoon => tr!(l;
            "Good afternoon", "午安", "下午好", "こんにちは", "좋은 오후",
            "Добрый день", "Bonjour", "Buenas tardes", "Guten Tag", "Chào buổi chiều",
            "Selamat siang", "Magandang hapon", "สวัสดีตอนบ่าย", "Goedemiddag", "İyi günler",
            "Boa tarde", "नमस्ते", "مساء الخير"),
        DayPart::Evening => tr!(l;
            "Good evening", "晚安", "晚上好", "こんばんは", "좋은 저녁",
            "Добрый вечер", "Bonsoir", "Buenas noches", "Guten Abend", "Chào buổi tối",
            "Selamat malam", "Magandang gabi", "สวัสดีตอนเย็น", "Goedenavond", "İyi akşamlar",
            "Boa noite", "शुभ संध्या", "مساء الخير"),
    }
}

/// The home-menu **body** — everything after the time-aware [`greeting`], starting
/// at the 👋. `menu::menu_text` composes `greeting + " " + intro`.
pub fn intro(l: Lang) -> &'static str {
    tr!(l;
        "👋 I'm Wixy.\nThe World Cup is live. Make your call and prove you know ball ⚽\nCheck in, Predict. Earn coins 👇",
        "👋 我是 Wixy。\n世界盃正熱烈開打！下好離手，證明你最懂球 ⚽\n簽到、預測，賺取金幣 👇",
        "👋 我是 Wixy。\n世界杯正火热开打！下好离手，证明你最懂球 ⚽\n签到、预测，赚取金币 👇",
        "👋 わたしは Wixy。\nワールドカップ開催中！予想を立てて、サッカー通なところを見せて ⚽\nチェックイン、予測してコインを稼ごう 👇",
        "👋 나는 Wixy야.\n월드컵이 한창이야. 예측을 걸고 축구 실력을 증명해 봐 ⚽\n출석하고 예측해서 코인을 벌자 👇",
        "👋 Я Wixy.\nЧемпионат мира в разгаре. Сделай прогноз и докажи, что разбираешься в футболе ⚽\nОтмечайся, предсказывай, зарабатывай монеты 👇",
        "👋 Je suis Wixy.\nLa Coupe du monde est lancée. Fais ton pronostic et prouve que tu t'y connais en foot ⚽\nPointe-toi, prédis, gagne des pièces 👇",
        "👋 Soy Wixy.\nEl Mundial está en marcha. Haz tu pronóstico y demuestra que sabes de fútbol ⚽\nRegístrate, predice y gana monedas 👇",
        "👋 Ich bin Wixy.\nDie WM läuft. Tipp ab und beweise, dass du was von Fußball verstehst ⚽\nEinchecken, tippen, Coins verdienen 👇",
        "👋 Mình là Wixy.\nWorld Cup đang diễn ra. Đặt kèo và chứng minh bạn rành bóng đá ⚽\nĐiểm danh, dự đoán, kiếm xu 👇",
        "👋 Aku Wixy.\nPiala Dunia sedang berlangsung. Buat tebakanmu dan buktikan kamu paham bola ⚽\nCheck-in, prediksi, kumpulkan koin 👇",
        "👋 Ako si Wixy.\nBuhay ang World Cup. Itaya ang hula mo at patunayang marunong ka sa bola ⚽\nMag-check-in, mag-predict, kumita ng coins 👇",
        "👋 ฉันชื่อ Wixy\nฟุตบอลโลกกำลังแข่งอยู่ ทายผลแล้วพิสูจน์ว่าคุณรู้จริงเรื่องบอล ⚽\nเช็คอิน ทายผล รับเหรียญ 👇",
        "👋 Ik ben Wixy.\nHet WK is bezig. Doe je voorspelling en bewijs dat je verstand van voetbal hebt ⚽\nCheck in, voorspel, verdien munten 👇",
        "👋 Ben Wixy.\nDünya Kupası başladı. Tahminini yap ve topdan anladığını kanıtla ⚽\nGiriş yap, tahmin et, coin kazan 👇",
        "👋 Eu sou a Wixy.\nA Copa do Mundo está rolando. Faça seu palpite e prove que manja de futebol ⚽\nFaça check-in, preveja e ganhe moedas 👇",
        "👋 मैं Wixy हूँ।\nवर्ल्ड कप जारी है। अपनी भविष्यवाणी करो और साबित करो कि तुम फ़ुटबॉल के उस्ताद हो ⚽\nचेक-इन करो, भविष्यवाणी करो, सिक्के कमाओ 👇",
        "👋 أنا Wixy.\nكأس العالم مستمرة. توقّع النتيجة وأثبت أنك تفهم في كرة القدم ⚽\nسجّل حضورك وتوقّع واكسب العملات 👇")
}

/// Closing prompt shown as the last line of the menu, after the status block.
/// Balance summary shown under the menu intro. (Fruit is hidden until the
/// fruit feature is designed.)
pub fn menu_status(l: Lang, coins: &str) -> String {
    tr!(l;
        "🪙 Balance: {coins}", "🪙 餘額：{coins}", "🪙 余额：{coins}", "🪙 残高：{coins}", "🪙 잔액: {coins}",
        "🪙 Баланс: {coins}", "🪙 Solde : {coins}", "🪙 Saldo: {coins}", "🪙 Guthaben: {coins}", "🪙 Số dư: {coins}",
        "🪙 Saldo: {coins}", "🪙 Balanse: {coins}", "🪙 ยอดเงิน: {coins}", "🪙 Saldo: {coins}", "🪙 Bakiye: {coins}",
        "🪙 Saldo: {coins}", "🪙 बैलेंस: {coins}", "🪙 الرصيد: {coins}")
    .replace("{coins}", coins)
}

pub fn btn_checkin(l: Lang) -> &'static str {
    tr!(l;
        "🪙 Daily check-in", "🪙 每日簽到", "🪙 每日签到", "🪙 デイリーチェックイン", "🪙 데일리 출석",
        "🪙 Ежедневный бонус", "🪙 Pointage du jour", "🪙 Registro diario", "🪙 Täglich einchecken", "🪙 Điểm danh hằng ngày",
        "🪙 Check-in harian", "🪙 Araw-araw na check-in", "🪙 เช็คอินประจำวัน", "🪙 Dagelijks inchecken", "🪙 Günlük giriş",
        "🪙 Check-in diário", "🪙 दैनिक चेक-इन", "🪙 تسجيل يومي")
}

pub fn btn_balance(l: Lang) -> &'static str {
    tr!(l;
        "💰 Check assets", "💰 查看資產", "💰 查看资产", "💰 資産を確認", "💰 자산 확인",
        "💰 Проверить активы", "💰 Voir les actifs", "💰 Ver activos", "💰 Vermögen ansehen", "💰 Xem tài sản",
        "💰 Cek aset", "💰 Tingnan ang assets", "💰 ดูสินทรัพย์", "💰 Activa bekijken", "💰 Varlıkları gör",
        "💰 Ver ativos", "💰 संपत्ति देखें", "💰 عرض الأصول")
}

pub fn btn_history(l: Lang) -> &'static str {
    tr!(l;
        "🧾 History", "🧾 歷史紀錄", "🧾 历史记录", "🧾 履歴", "🧾 기록",
        "🧾 История", "🧾 Historique", "🧾 Historial", "🧾 Verlauf", "🧾 Lịch sử",
        "🧾 Riwayat", "🧾 Kasaysayan", "🧾 ประวัติ", "🧾 Geschiedenis", "🧾 Geçmiş",
        "🧾 Histórico", "🧾 इतिहास", "🧾 السجل")
}

pub fn btn_markets(l: Lang) -> &'static str {
    tr!(l;
        "⚽ Today's markets", "⚽ 今日市場", "⚽ 今日市场", "⚽ 今日のマーケット", "⚽ 오늘의 마켓",
        "⚽ Рынки сегодня", "⚽ Marchés du jour", "⚽ Mercados de hoy", "⚽ Heutige Märkte", "⚽ Thị trường hôm nay",
        "⚽ Pasar hari ini", "⚽ Mga market ngayon", "⚽ ตลาดวันนี้", "⚽ Markten vandaag", "⚽ Bugünkü piyasalar",
        "⚽ Mercados de hoje", "⚽ आज के मार्केट", "⚽ أسواق اليوم")
}

pub fn btn_rule(l: Lang) -> &'static str {
    tr!(l;
        "📜 How to earn coins", "📜 如何賺幣", "📜 如何赚金币", "📜 コインの稼ぎ方", "📜 코인 버는 법",
        "📜 Как заработать монеты", "📜 Comment gagner des pièces", "📜 Cómo ganar monedas", "📜 Münzen verdienen", "📜 Cách kiếm xu",
        "📜 Cara mendapat koin", "📜 Paano kumita ng coins", "📜 วิธีหาเหรียญ", "📜 Munten verdienen", "📜 Para nasıl kazanılır",
        "📜 Como ganhar moedas", "📜 कॉइन कैसे कमाएँ", "📜 كيفية كسب العملات")
}

/// Home-page button (groups only) that kicks off the `/predict` builder.
pub fn btn_predict(l: Lang) -> &'static str {
    tr!(l;
        "🎲 Create prediction", "🎲 發起預測", "🎲 发起预测", "🎲 予測を作成", "🎲 예측 만들기",
        "🎲 Создать прогноз", "🎲 Créer une prédiction", "🎲 Crear predicción", "🎲 Vorhersage erstellen", "🎲 Tạo dự đoán",
        "🎲 Buat prediksi", "🎲 Gumawa ng prediksyon", "🎲 สร้างคำทำนาย", "🎲 Voorspelling maken", "🎲 Tahmin oluştur",
        "🎲 Criar previsão", "🎲 भविष्यवाणी बनाएं", "🎲 إنشاء توقّع")
}

/// Group home-page button: repost a prediction card the host deleted from the chat.
pub fn btn_restore_card(l: Lang) -> &'static str {
    tr!(l;
        "♻️ Restore card", "♻️ 還原卡片", "♻️ 还原卡片", "♻️ カードを復元", "♻️ 카드 복원",
        "♻️ Восстановить карточку", "♻️ Restaurer la carte", "♻️ Restaurar tarjeta", "♻️ Karte wiederherstellen", "♻️ Khôi phục thẻ",
        "♻️ Pulihkan kartu", "♻️ Ibalik ang card", "♻️ กู้คืนการ์ด", "♻️ Kaart herstellen", "♻️ Kartı geri yükle",
        "♻️ Restaurar cartão", "♻️ कार्ड बहाल करें", "♻️ استعادة البطاقة")
}

/// Toast when the host taps the in-group restore button but hosts no open
/// prediction in this group (nothing to repost here).
pub fn nothing_to_restore(l: Lang) -> &'static str {
    tr!(l;
        "You have no prediction card to restore in this group.", "你在這個群組沒有可還原的預測卡片。", "你在这个群组没有可还原的预测卡片。", "このグループに復元できる予測カードはありません。", "이 그룹에 복원할 예측 카드가 없습니다.",
        "У вас нет карточки прогноза для восстановления в этой группе.", "Tu n'as aucune carte de prédiction à restaurer dans ce groupe.", "No tienes ninguna tarjeta de predicción para restaurar en este grupo.", "Du hast in dieser Gruppe keine Vorhersagekarte zum Wiederherstellen.", "Bạn không có thẻ dự đoán nào để khôi phục trong nhóm này.",
        "Kamu tidak punya kartu prediksi untuk dipulihkan di grup ini.", "Wala kang prediction card na maibabalik sa grupong ito.", "คุณไม่มีการ์ดคำทำนายให้กู้คืนในกลุ่มนี้", "Je hebt geen voorspellingskaart om in deze groep te herstellen.", "Bu grupta geri yükleyecek bir tahmin kartın yok.",
        "Você não tem nenhum cartão de previsão para restaurar neste grupo.", "इस समूह में बहाल करने के लिए आपके पास कोई भविष्यवाणी कार्ड नहीं है।", "ليس لديك بطاقة توقّع لاستعادتها في هذه المجموعة.")
}

pub fn btn_invite(l: Lang) -> &'static str {
    tr!(l;
        "🔗 Invite friends", "🔗 邀請好友", "🔗 邀请好友", "🔗 友達を招待", "🔗 친구 초대",
        "🔗 Пригласить друзей", "🔗 Inviter des amis", "🔗 Invitar amigos", "🔗 Freunde einladen", "🔗 Mời bạn bè",
        "🔗 Undang teman", "🔗 Mag-imbita ng kaibigan", "🔗 ชวนเพื่อน", "🔗 Vrienden uitnodigen", "🔗 Arkadaş davet et",
        "🔗 Convidar amigos", "🔗 दोस्तों को बुलाएं", "🔗 ادعُ أصدقاءك")
}

/// `/onlyreplyhere` confirmation: the bot is now confined to this topic.
pub fn onlyreply_set(l: Lang) -> &'static str {
    tr!(l;
        "✅ Got it — I'll only reply in this topic from now on.", "✅ 好的，我之後只會在這個主題裡回覆。", "✅ 好的，我之后只会在这个话题里回复。", "✅ 了解しました。これからはこのトピックでのみ返信します。", "✅ 알겠어요. 이제부터 이 토픽에서만 답할게요.",
        "✅ Понятно — теперь я отвечаю только в этой теме.", "✅ Compris — je ne répondrai plus que dans ce sujet.", "✅ Entendido: a partir de ahora solo responderé en este tema.", "✅ Verstanden – ich antworte ab jetzt nur in diesem Thema.", "✅ Đã hiểu — từ giờ tôi chỉ trả lời trong chủ đề này.",
        "✅ Oke — mulai sekarang saya hanya membalas di topik ini.", "✅ Sige — mula ngayon sa topic na ito na lang ako sasagot.", "✅ รับทราบ — ต่อจากนี้ฉันจะตอบเฉพาะในหัวข้อนี้เท่านั้น", "✅ Begrepen — ik antwoord voortaan alleen in dit onderwerp.", "✅ Anlaşıldı — bundan sonra yalnızca bu konuda yanıt vereceğim.",
        "✅ Entendi — a partir de agora só responderei neste tópico.", "✅ ठीक है — अब से मैं केवल इसी टॉपिक में जवाब दूंगा।", "✅ تمام — من الآن سأرد في هذا الموضوع فقط.")
}

/// `/replyanywhere` confirmation: the topic lock is cleared.
pub fn onlyreply_cleared(l: Lang) -> &'static str {
    tr!(l;
        "✅ Done — I'll reply in any topic again.", "✅ 完成，我又可以在任何主題裡回覆了。", "✅ 完成，我又可以在任何话题里回复了。", "✅ 完了。またどのトピックでも返信します。", "✅ 완료. 이제 어떤 토픽에서도 다시 답할게요.",
        "✅ Готово — я снова отвечаю в любой теме.", "✅ C'est fait — je réponds de nouveau dans tous les sujets.", "✅ Listo: volveré a responder en cualquier tema.", "✅ Erledigt – ich antworte wieder in jedem Thema.", "✅ Xong — tôi sẽ trả lời trong mọi chủ đề trở lại.",
        "✅ Selesai — saya akan membalas di topik mana pun lagi.", "✅ Tapos na — sasagot na ulit ako sa kahit anong topic.", "✅ เรียบร้อย — ฉันจะตอบในทุกหัวข้ออีกครั้ง", "✅ Klaar — ik antwoord weer in elk onderwerp.", "✅ Tamam — yine her konuda yanıt vereceğim.",
        "✅ Pronto — voltarei a responder em qualquer tópico.", "✅ हो गया — अब मैं फिर से किसी भी टॉपिक में जवाब दूंगा।", "✅ تم — سأرد في أي موضوع مرة أخرى.")
}

/// `/onlyreplyhere` used outside a forum topic — there's nothing to lock to.
pub fn onlyreply_need_topic(l: Lang) -> &'static str {
    tr!(l;
        "Use this inside a topic, so I know where to stay.", "請在某個主題裡使用這個指令，我才知道要待在哪裡。", "请在某个话题里使用这个指令，我才知道要待在哪里。", "どのトピックに留まればいいか分かるよう、トピック内でこのコマンドを使ってください。", "어느 토픽에 머무를지 알 수 있도록 토픽 안에서 이 명령을 사용해 주세요.",
        "Используйте это внутри темы, чтобы я знал, где оставаться.", "Utilisez ceci dans un sujet, pour que je sache où rester.", "Usa esto dentro de un tema para que sepa dónde quedarme.", "Verwende das innerhalb eines Themas, damit ich weiß, wo ich bleiben soll.", "Hãy dùng lệnh này trong một chủ đề để tôi biết nên ở đâu.",
        "Gunakan ini di dalam sebuah topik, supaya saya tahu harus tinggal di mana.", "Gamitin ito sa loob ng isang topic para malaman ko kung saan ako mananatili.", "ใช้คำสั่งนี้ภายในหัวข้อ เพื่อให้ฉันรู้ว่าควรอยู่ที่ไหน", "Gebruik dit binnen een onderwerp, zodat ik weet waar ik moet blijven.", "Bunu bir konunun içinde kullan ki nerede kalacağımı bileyim.",
        "Use isto dentro de um tópico, para eu saber onde ficar.", "इसे किसी टॉपिक के अंदर इस्तेमाल करें, ताकि मुझे पता चले कि कहाँ रहना है।", "استخدم هذا داخل موضوع حتى أعرف أين أبقى.")
}

/// A non-admin tried to change the topic lock.
pub fn onlyreply_admin_only(l: Lang) -> &'static str {
    tr!(l;
        "Only group admins can change this.", "只有群組管理員可以更改這個設定。", "只有群组管理员可以更改这个设置。", "この設定を変更できるのはグループ管理者だけです。", "이 설정은 그룹 관리자만 바꿀 수 있어요.",
        "Это могут менять только администраторы группы.", "Seuls les admins du groupe peuvent changer ceci.", "Solo los administradores del grupo pueden cambiar esto.", "Nur Gruppen-Admins können das ändern.", "Chỉ quản trị viên nhóm mới có thể thay đổi điều này.",
        "Hanya admin grup yang bisa mengubah ini.", "Mga admin lang ng grupo ang puwedeng magbago nito.", "เฉพาะแอดมินกลุ่มเท่านั้นที่เปลี่ยนได้", "Alleen groepsbeheerders kunnen dit wijzigen.", "Bunu yalnızca grup yöneticileri değiştirebilir.",
        "Apenas administradores do grupo podem alterar isto.", "इसे केवल ग्रुप एडमिन बदल सकते हैं।", "يمكن لمشرفي المجموعة فقط تغيير هذا.")
}

/// `/onlyreplyhere`/`/replyanywhere` used outside a group.
pub fn onlyreply_group_only(l: Lang) -> &'static str {
    tr!(l;
        "This only works inside a group.", "這個指令只能在群組裡使用。", "这个指令只能在群组里使用。", "これはグループ内でのみ使えます。", "이건 그룹 안에서만 작동해요.",
        "Это работает только в группе.", "Ceci ne fonctionne que dans un groupe.", "Esto solo funciona dentro de un grupo.", "Das funktioniert nur in einer Gruppe.", "Lệnh này chỉ hoạt động trong nhóm.",
        "Ini hanya berfungsi di dalam grup.", "Gumagana lang ito sa loob ng grupo.", "ใช้ได้เฉพาะภายในกลุ่มเท่านั้น", "Dit werkt alleen binnen een groep.", "Bu yalnızca bir grup içinde çalışır.",
        "Isto só funciona dentro de um grupo.", "यह केवल ग्रुप के अंदर काम करता है।", "هذا يعمل داخل المجموعة فقط.")
}

/// Forward-safe deep-link button on the home page (URL button → survives
/// forwarding; a recipient who taps it joins via the sharer's referral link).
pub fn btn_join(l: Lang) -> &'static str {
    tr!(l;
        "🎮 Play now", "🎮 立即遊玩", "🎮 立即游玩", "🎮 今すぐプレイ", "🎮 지금 플레이",
        "🎮 Играть", "🎮 Jouer", "🎮 Jugar", "🎮 Jetzt spielen", "🎮 Chơi ngay",
        "🎮 Main sekarang", "🎮 Maglaro", "🎮 เล่นเลย", "🎮 Speel nu", "🎮 Hemen oyna",
        "🎮 Jogar agora", "🎮 अभी खेलें", "🎮 العب الآن")
}

/// Invite-link message: the user's referral link plus how many they've referred.
/// Referral count line, shown above the invite-format chooser.
pub fn invite_count(l: Lang, count: &str) -> String {
    tr!(l;
        "Your referrals: {count} 🎉", "你已邀請：{count} 人 🎉", "你已邀请：{count} 人 🎉", "招待数：{count} 🎉", "초대 수: {count} 🎉",
        "Приглашено: {count} 🎉", "Tes parrainages : {count} 🎉", "Tus referidos: {count} 🎉", "Deine Einladungen: {count} 🎉", "Đã mời: {count} 🎉",
        "Referralmu: {count} 🎉", "Iyong referrals: {count} 🎉", "ผู้ที่คุณชวนมา: {count} 🎉", "Jouw verwijzingen: {count} 🎉", "Davetlerin: {count} 🎉",
        "Suas indicações: {count} 🎉", "आपके रेफ़रल: {count} 🎉", "إحالاتك: {count} 🎉")
    .replace("{count}", count)
}

/// Chooser shown when the user taps [Invite friends]: pick a share format.
pub fn invite_how(l: Lang) -> &'static str {
    tr!(l;
        "How do you want to share your invite? 👇", "你想怎麼分享邀請？👇", "你想怎么分享邀请？👇", "どの方法で招待を共有する？👇", "어떻게 초대를 공유할까요? 👇",
        "Как поделиться приглашением? 👇", "Comment partager ton invitation ? 👇", "¿Cómo quieres compartir tu invitación? 👇", "Wie möchtest du deine Einladung teilen? 👇", "Bạn muốn chia sẻ lời mời thế nào? 👇",
        "Bagaimana kamu ingin membagikan undangan? 👇", "Paano mo gustong ibahagi ang imbitasyon? 👇", "อยากแชร์คำเชิญแบบไหน? 👇", "Hoe wil je je uitnodiging delen? 👇", "Davetini nasıl paylaşmak istersin? 👇",
        "Como você quer compartilhar seu convite? 👇", "आप अपना निमंत्रण कैसे साझा करना चाहते हैं? 👇", "كيف تريد مشاركة دعوتك؟ 👇")
}

pub fn btn_invite_link(l: Lang) -> &'static str {
    tr!(l;
        "🔗 Copyable link", "🔗 可複製連結", "🔗 可复制链接", "🔗 コピー用リンク", "🔗 복사용 링크",
        "🔗 Ссылка для копирования", "🔗 Lien à copier", "🔗 Enlace para copiar", "🔗 Link zum Kopieren", "🔗 Liên kết sao chép",
        "🔗 Tautan salin", "🔗 Kopyahin ang link", "🔗 ลิงก์คัดลอก", "🔗 Kopieerbare link", "🔗 Kopyalanabilir bağlantı",
        "🔗 Link para copiar", "🔗 कॉपी करने योग्य लिंक", "🔗 رابط قابل للنسخ")
}

pub fn btn_invite_fwd(l: Lang) -> &'static str {
    tr!(l;
        "📤 Forwardable message", "📤 可轉發訊息", "📤 可转发消息", "📤 転送用メッセージ", "📤 전달용 메시지",
        "📤 Сообщение для пересылки", "📤 Message à transférer", "📤 Mensaje para reenviar", "📤 Weiterleitbare Nachricht", "📤 Tin nhắn để chuyển tiếp",
        "📤 Pesan untuk diteruskan", "📤 Maipapasa na mensahe", "📤 ข้อความส่งต่อ", "📤 Doorstuurbaar bericht", "📤 İletilebilir mesaj",
        "📤 Mensagem para encaminhar", "📤 फ़ॉरवर्ड करने योग्य संदेश", "📤 رسالة قابلة لإعادة التوجيه")
}

pub fn btn_invite_qr(l: Lang) -> &'static str {
    tr!(l;
        "🔳 QR code", "🔳 QR 碼", "🔳 二维码", "🔳 QRコード", "🔳 QR 코드",
        "🔳 QR-код", "🔳 Code QR", "🔳 Código QR", "🔳 QR-Code", "🔳 Mã QR",
        "🔳 Kode QR", "🔳 QR code", "🔳 คิวอาร์โค้ด", "🔳 QR-code", "🔳 QR kodu",
        "🔳 Código QR", "🔳 QR कोड", "🔳 رمز QR")
}

/// Home-page badge (private only) shown when the host has open predictions to
/// settle; `{n}` is the count.
pub fn btn_settle_pending(l: Lang, n: usize) -> String {
    tr!(l;
        "⚙️ Settle predictions ({n})", "⚙️ 結算預測 ({n})", "⚙️ 结算预测 ({n})", "⚙️ 予測を精算 ({n})", "⚙️ 예측 정산 ({n})",
        "⚙️ Рассчитать прогнозы ({n})", "⚙️ Régler les prédictions ({n})", "⚙️ Liquidar predicciones ({n})", "⚙️ Vorhersagen abrechnen ({n})", "⚙️ Quyết toán dự đoán ({n})",
        "⚙️ Selesaikan prediksi ({n})", "⚙️ Ayusin ang prediksyon ({n})", "⚙️ ปิดผลคำทำนาย ({n})", "⚙️ Voorspellingen afrekenen ({n})", "⚙️ Tahminleri sonuçlandır ({n})",
        "⚙️ Liquidar previsões ({n})", "⚙️ भविष्यवाणियाँ निपटाएँ ({n})", "⚙️ تسوية التوقّعات ({n})")
    .replace("{n}", &n.to_string())
}

/// Header of the `menu:settle` list — the host's open predictions.
pub fn settle_list_title(l: Lang) -> &'static str {
    tr!(l;
        "⚙️ Predictions you're hosting — tap one to open and settle it:", "⚙️ 你發起的預測 — 點一個開啟並結算：", "⚙️ 你发起的预测 — 点一个打开并结算：", "⚙️ あなたが主催中の予測 — タップして開き精算しましょう：", "⚙️ 내가 진행 중인 예측 — 눌러서 열고 정산하세요:",
        "⚙️ Ваши прогнозы — нажмите, чтобы открыть и рассчитать:", "⚙️ Tes prédictions en cours — touche pour l'ouvrir et la régler :", "⚙️ Tus predicciones en curso — toca una para abrirla y liquidarla:", "⚙️ Deine laufenden Vorhersagen — zum Öffnen und Abrechnen tippen:", "⚙️ Dự đoán bạn đang tổ chức — chạm để mở và quyết toán:",
        "⚙️ Prediksi yang kamu adakan — ketuk untuk membuka dan menyelesaikan:", "⚙️ Mga prediksyong hino-host mo — i-tap para buksan at ayusin:", "⚙️ คำทำนายที่คุณจัด — แตะเพื่อเปิดและปิดผล:", "⚙️ Voorspellingen die je host — tik om te openen en af te rekenen:", "⚙️ Düzenlediğin tahminler — açıp sonuçlandırmak için dokun:",
        "⚙️ Previsões que você criou — toque para abrir e liquidar:", "⚙️ आपकी चल रही भविष्यवाणियाँ — खोलकर निपटाने के लिए टैप करें:", "⚙️ التوقّعات التي تديرها — انقر لفتحها وتسويتها:")
}

/// Button on the per-prediction settle screen that reposts a fresh copy of the
/// card to the origin chat (for when the original was deleted from the chat).
pub fn repost_card_btn(l: Lang) -> &'static str {
    tr!(l;
        "♻️ Repost card", "♻️ 重新發送卡片", "♻️ 重新发送卡片", "♻️ カードを再投稿", "♻️ 카드 다시 게시",
        "♻️ Опубликовать заново", "♻️ Republier la carte", "♻️ Republicar tarjeta", "♻️ Karte erneut posten", "♻️ Đăng lại thẻ",
        "♻️ Kirim ulang kartu", "♻️ I-repost ang card", "♻️ โพสต์การ์ดใหม่", "♻️ Kaart opnieuw plaatsen", "♻️ Kartı yeniden gönder",
        "♻️ Repostar cartão", "♻️ कार्ड फिर से पोस्ट करें", "♻️ إعادة نشر البطاقة")
}

/// Toast confirming the prediction card was reposted to the origin chat.
pub fn card_reposted(l: Lang) -> &'static str {
    tr!(l;
        "♻️ Card reposted.", "♻️ 已重新發送卡片。", "♻️ 已重新发送卡片。", "♻️ カードを再投稿しました。", "♻️ 카드를 다시 게시했습니다.",
        "♻️ Карточка опубликована заново.", "♻️ Carte republiée.", "♻️ Tarjeta republicada.", "♻️ Karte erneut gepostet.", "♻️ Đã đăng lại thẻ.",
        "♻️ Kartu dikirim ulang.", "♻️ Na-repost ang card.", "♻️ โพสต์การ์ดใหม่แล้ว", "♻️ Kaart opnieuw geplaatst.", "♻️ Kart yeniden gönderildi.",
        "♻️ Cartão repostado.", "♻️ कार्ड फिर से पोस्ट किया गया।", "♻️ تمت إعادة نشر البطاقة.")
}

/// Screen shown when a host taps a prediction that isn't settleable yet (deadline
/// still in the future); `{when}` = the deadline in the host's local time. Offers
/// the repost option below.
pub fn settle_not_ready(l: Lang, when: &str) -> String {
    tr!(l;
        "🟢 Still open until {when} — you can't settle this yet.\nIf the card was deleted from the chat, repost it below.", "🟢 仍開放至 {when} — 尚無法結算。\n若卡片已從聊天刪除，可在下方重新發送。", "🟢 仍开放至 {when} — 尚无法结算。\n若卡片已从聊天删除，可在下方重新发送。", "🟢 {when} まで受付中 — まだ精算できません。\nカードがチャットから削除された場合は、下から再投稿できます。", "🟢 {when}까지 진행 중 — 아직 정산할 수 없습니다.\n카드가 채팅에서 삭제됐다면 아래에서 다시 게시하세요.",
        "🟢 Открыто до {when} — рассчитать ещё нельзя.\nЕсли карточку удалили из чата, опубликуйте её заново ниже.", "🟢 Ouvert jusqu'à {when} — pas encore réglable.\nSi la carte a été supprimée du chat, republie-la ci-dessous.", "🟢 Abierto hasta {when} — aún no puedes liquidarla.\nSi la tarjeta se eliminó del chat, repúblicala abajo.", "🟢 Offen bis {when} — noch nicht abrechenbar.\nWurde die Karte aus dem Chat gelöscht, poste sie unten erneut.", "🟢 Mở đến {when} — chưa thể quyết toán.\nNếu thẻ đã bị xóa khỏi cuộc trò chuyện, hãy đăng lại bên dưới.",
        "🟢 Masih buka sampai {when} — belum bisa diselesaikan.\nJika kartu terhapus dari obrolan, kirim ulang di bawah.", "🟢 Bukas pa hanggang {when} — hindi mo pa ito maaayos.\nKung nabura ang card sa chat, i-repost ito sa ibaba.", "🟢 ยังเปิดถึง {when} — ยังปิดผลไม่ได้\nถ้าการ์ดถูกลบจากแชท สามารถโพสต์ใหม่ด้านล่าง", "🟢 Open tot {when} — nog niet af te rekenen.\nIs de kaart uit de chat verwijderd, plaats hem hieronder opnieuw.", "🟢 {when} kadar açık — henüz sonuçlandıramazsın.\nKart sohbetten silindiyse aşağıdan yeniden gönder.",
        "🟢 Aberto até {when} — ainda não dá para liquidar.\nSe o cartão foi apagado do chat, reposte-o abaixo.", "🟢 {when} तक खुला — अभी निपटा नहीं सकते।\nअगर कार्ड चैट से हटा दिया गया है, तो उसे नीचे फिर से पोस्ट करें।", "🟢 مفتوح حتى {when} — لا يمكن التسوية بعد.\nإذا حُذفت البطاقة من المحادثة، فأعد نشرها بالأسفل.")
    .replace("{when}", when)
}

/// Title of the per-prediction outcome picker reached from the settle list.
pub fn settle_pick_outcome(l: Lang) -> &'static str {
    tr!(l;
        "🏁 Who won? Pick the winning outcome:", "🏁 誰贏了？選擇結算的選項：", "🏁 谁赢了？选择结算的选项：", "🏁 結果は？勝った選択肢を選んでください：", "🏁 누가 이겼나요? 정산할 결과를 선택하세요:",
        "🏁 Кто выиграл? Выберите победивший исход:", "🏁 Qui a gagné ? Choisis l'issue gagnante :", "🏁 ¿Quién ganó? Elige el resultado ganador:", "🏁 Wer hat gewonnen? Wähle das gewinnende Ergebnis:", "🏁 Ai thắng? Chọn kết quả thắng:",
        "🏁 Siapa yang menang? Pilih hasil yang menang:", "🏁 Sino ang nanalo? Piliin ang nanalong resulta:", "🏁 ใครชนะ? เลือกผลที่ชนะ:", "🏁 Wie heeft gewonnen? Kies de winnende uitkomst:", "🏁 Kim kazandı? Kazanan sonucu seç:",
        "🏁 Quem ganhou? Escolha o resultado vencedor:", "🏁 कौन जीता? जीतने वाला विकल्प चुनें:", "🏁 من فاز؟ اختر النتيجة الفائزة:")
}

/// First of two confirmations before settling a prediction (`{outcome}` = winner).
pub fn settle_confirm1(l: Lang, outcome: &str) -> String {
    tr!(l;
        "Settle as “{outcome}”?\nWinners will be paid out.\n\n(confirm 1 of 2)", "確定結算為「{outcome}」？\n贏家將獲得派彩。\n\n（確認 1/2）", "确定结算为「{outcome}」？\n赢家将获得派彩。\n\n（确认 1/2）", "「{outcome}」で精算しますか？\n勝者に配当が支払われます。\n\n（確認 1/2）", "“{outcome}”(으)로 정산할까요?\n승자에게 지급됩니다.\n\n(확인 1/2)",
        "Рассчитать как «{outcome}»?\nПобедителям будет выплачено.\n\n(подтверждение 1 из 2)", "Régler sur « {outcome} » ?\nLes gagnants seront payés.\n\n(confirmation 1 sur 2)", "¿Liquidar como «{outcome}»?\nSe pagará a los ganadores.\n\n(confirmación 1 de 2)", "Als „{outcome}“ abrechnen?\nGewinner werden ausgezahlt.\n\n(Bestätigung 1 von 2)", "Quyết toán là “{outcome}”?\nNgười thắng sẽ được trả thưởng.\n\n(xác nhận 1/2)",
        "Selesaikan sebagai “{outcome}”?\nPemenang akan dibayar.\n\n(konfirmasi 1 dari 2)", "Ayusin bilang “{outcome}”?\nBabayaran ang mga nanalo.\n\n(kumpirmasyon 1 ng 2)", "ปิดผลเป็น “{outcome}” ไหม?\nผู้ชนะจะได้รับเงินรางวัล\n\n(ยืนยัน 1 จาก 2)", "Afrekenen als “{outcome}”?\nWinnaars worden uitbetaald.\n\n(bevestiging 1 van 2)", "“{outcome}” olarak sonuçlandırılsın mı?\nKazananlara ödeme yapılacak.\n\n(onay 1 / 2)",
        "Liquidar como “{outcome}”?\nOs vencedores serão pagos.\n\n(confirmação 1 de 2)", "“{outcome}” के रूप में निपटाएं?\nविजेताओं को भुगतान किया जाएगा।\n\n(पुष्टि 1 / 2)", "التسوية كـ «{outcome}»؟\nسيتم الدفع للفائزين.\n\n(تأكيد 1 من 2)")
    .replace("{outcome}", outcome)
}

/// Final confirmation before settling a prediction (`{outcome}` = winner).
pub fn settle_confirm2(l: Lang, outcome: &str) -> String {
    tr!(l;
        "⚠️ Final check — this can't be undone.\nSettle as “{outcome}”?\n\n(confirm 2 of 2)", "⚠️ 最後確認 — 無法撤銷。\n結算為「{outcome}」？\n\n（確認 2/2）", "⚠️ 最后确认 — 无法撤销。\n结算为「{outcome}」？\n\n（确认 2/2）", "⚠️ 最終確認 — 取り消せません。\n「{outcome}」で精算しますか？\n\n（確認 2/2）", "⚠️ 최종 확인 — 되돌릴 수 없습니다.\n“{outcome}”(으)로 정산할까요?\n\n(확인 2/2)",
        "⚠️ Последняя проверка — отменить нельзя.\nРассчитать как «{outcome}»?\n\n(подтверждение 2 из 2)", "⚠️ Dernière vérification — irréversible.\nRégler sur « {outcome} » ?\n\n(confirmation 2 sur 2)", "⚠️ Última comprobación — no se puede deshacer.\n¿Liquidar como «{outcome}»?\n\n(confirmación 2 de 2)", "⚠️ Letzte Prüfung — nicht rückgängig zu machen.\nAls „{outcome}“ abrechnen?\n\n(Bestätigung 2 von 2)", "⚠️ Kiểm tra cuối — không thể hoàn tác.\nQuyết toán là “{outcome}”?\n\n(xác nhận 2/2)",
        "⚠️ Pemeriksaan akhir — tidak bisa dibatalkan.\nSelesaikan sebagai “{outcome}”?\n\n(konfirmasi 2 dari 2)", "⚠️ Huling tsek — hindi na mababawi.\nAyusin bilang “{outcome}”?\n\n(kumpirmasyon 2 ng 2)", "⚠️ ตรวจสอบครั้งสุดท้าย — ยกเลิกไม่ได้\nปิดผลเป็น “{outcome}” ไหม?\n\n(ยืนยัน 2 จาก 2)", "⚠️ Laatste check — kan niet ongedaan worden.\nAfrekenen als “{outcome}”?\n\n(bevestiging 2 van 2)", "⚠️ Son kontrol — geri alınamaz.\n“{outcome}” olarak sonuçlandırılsın mı?\n\n(onay 2 / 2)",
        "⚠️ Verificação final — não pode ser desfeita.\nLiquidar como “{outcome}”?\n\n(confirmação 2 de 2)", "⚠️ अंतिम जाँच — यह वापस नहीं किया जा सकता।\n“{outcome}” के रूप में निपटाएं?\n\n(पुष्टि 2 / 2)", "⚠️ تحقّق أخير — لا يمكن التراجع.\nالتسوية كـ «{outcome}»؟\n\n(تأكيد 2 من 2)")
    .replace("{outcome}", outcome)
}

/// Button: advance from the first confirmation to the final one.
pub fn settle_btn_continue(l: Lang) -> &'static str {
    tr!(l;
        "✅ Continue", "✅ 繼續", "✅ 继续", "✅ 続ける", "✅ 계속",
        "✅ Продолжить", "✅ Continuer", "✅ Continuar", "✅ Weiter", "✅ Tiếp tục",
        "✅ Lanjut", "✅ Magpatuloy", "✅ ดำเนินการต่อ", "✅ Doorgaan", "✅ Devam",
        "✅ Continuar", "✅ जारी रखें", "✅ متابعة")
}

/// Button: execute the settlement (final confirmation).
pub fn settle_btn_settle(l: Lang) -> &'static str {
    tr!(l;
        "⚠️ Settle now", "⚠️ 立即結算", "⚠️ 立即结算", "⚠️ 今すぐ精算", "⚠️ 지금 정산",
        "⚠️ Рассчитать", "⚠️ Régler maintenant", "⚠️ Liquidar ahora", "⚠️ Jetzt abrechnen", "⚠️ Quyết toán ngay",
        "⚠️ Selesaikan sekarang", "⚠️ Ayusin na", "⚠️ ปิดผลเลย", "⚠️ Nu afrekenen", "⚠️ Şimdi sonuçlandır",
        "⚠️ Liquidar agora", "⚠️ अभी निपटाएं", "⚠️ سوِّ الآن")
}

/// Copyable-link output (HTML `parse_mode`): the link in a `<code>` span renders
/// monospace and is tap-to-copy on Telegram clients.
pub fn invite_copy(l: Lang, link: &str) -> String {
    tr!(l;
        "Tap the link to copy it:\n<code>{link}</code>", "點擊連結即可複製：\n<code>{link}</code>", "点击链接即可复制：\n<code>{link}</code>", "リンクをタップしてコピー：\n<code>{link}</code>", "링크를 탭하여 복사:\n<code>{link}</code>",
        "Нажмите на ссылку, чтобы скопировать:\n<code>{link}</code>", "Touche le lien pour le copier :\n<code>{link}</code>", "Toca el enlace para copiarlo:\n<code>{link}</code>", "Tippe auf den Link, um ihn zu kopieren:\n<code>{link}</code>", "Chạm vào liên kết để sao chép:\n<code>{link}</code>",
        "Ketuk tautan untuk menyalin:\n<code>{link}</code>", "I-tap ang link para kopyahin:\n<code>{link}</code>", "แตะลิงก์เพื่อคัดลอก:\n<code>{link}</code>", "Tik op de link om te kopiëren:\n<code>{link}</code>", "Kopyalamak için bağlantıya dokun:\n<code>{link}</code>",
        "Toque no link para copiar:\n<code>{link}</code>", "लिंक कॉपी करने के लिए टैप करें:\n<code>{link}</code>", "انقر على الرابط لنسخه:\n<code>{link}</code>")
    .replace("{link}", link)
}

/// Forwardable message: friendly one-liner with the link in plain text, so it
/// survives a forward (inline keyboards don't).
pub fn invite_forward(l: Lang, link: &str) -> String {
    tr!(l;
        "🎮 Come play with me on Wixy!\n{link}", "🎮 來跟 Wixy 一起玩！\n{link}", "🎮 来跟 Wixy 一起玩！\n{link}", "🎮 Wixy で一緒に遊ぼう！\n{link}", "🎮 Wixy에서 같이 놀자!\n{link}",
        "🎮 Заходи играть со мной в Wixy!\n{link}", "🎮 Viens jouer avec moi sur Wixy !\n{link}", "🎮 ¡Ven a jugar conmigo en Wixy!\n{link}", "🎮 Komm und spiel mit mir auf Wixy!\n{link}", "🎮 Vào chơi với mình trên Wixy nhé!\n{link}",
        "🎮 Ayo main bareng aku di Wixy!\n{link}", "🎮 Halika't maglaro tayo sa Wixy!\n{link}", "🎮 มาเล่นกับฉันบน Wixy สิ!\n{link}", "🎮 Kom met me spelen op Wixy!\n{link}", "🎮 Gel Wixy'da birlikte oynayalım!\n{link}",
        "🎮 Venha jogar comigo no Wixy!\n{link}", "🎮 Wixy पर मेरे साथ खेलने आओ!\n{link}", "🎮 تعال نلعب معًا على Wixy!\n{link}")
    .replace("{link}", link)
}

/// DM to the referrer when a new user joins through their link.
pub fn referral_bonus(l: Lang, name: &str, amt: &str) -> String {
    tr!(l;
        "🎉 {name} joined via your link! +{amt} coins", "🎉 {name} 透過你的連結加入了！+{amt} 金幣", "🎉 {name} 通过你的链接加入了！+{amt} 金币", "🎉 {name} があなたのリンクから参加！+{amt} コイン", "🎉 {name} 님이 내 링크로 가입! +{amt} 코인",
        "🎉 {name} присоединился по вашей ссылке! +{amt} монет", "🎉 {name} a rejoint via ton lien ! +{amt} pièces", "🎉 ¡{name} se unió con tu enlace! +{amt} monedas", "🎉 {name} ist über deinen Link beigetreten! +{amt} Münzen", "🎉 {name} đã tham gia qua liên kết của bạn! +{amt} xu",
        "🎉 {name} bergabung lewat tautanmu! +{amt} koin", "🎉 Sumali si {name} gamit ang link mo! +{amt} coins", "🎉 {name} เข้าร่วมผ่านลิงก์ของคุณ! +{amt} เหรียญ", "🎉 {name} is via jouw link lid geworden! +{amt} munten", "🎉 {name} bağlantınla katıldı! +{amt} para",
        "🎉 {name} entrou pelo seu link! +{amt} moedas", "🎉 {name} आपके लिंक से जुड़े! +{amt} कॉइन", "🎉 انضم {name} عبر رابطك! +{amt} عملة")
    .replace("{name}", name)
    .replace("{amt}", amt)
}

// ----------------------------------------------------------------------------
// `/checkin` daily reward
// ----------------------------------------------------------------------------

pub fn checkin_done(l: Lang, amt: &str) -> String {
    tr!(l;
        "Checked in! +{amt} coins 🪙", "簽到成功！+{amt} 金幣 🪙", "签到成功！+{amt} 金币 🪙", "チェックイン完了！+{amt} コイン 🪙", "출석 완료! +{amt} 코인 🪙",
        "Отметка получена! +{amt} монет 🪙", "Pointage validé ! +{amt} pièces 🪙", "¡Registrado! +{amt} monedas 🪙", "Eingecheckt! +{amt} Münzen 🪙", "Điểm danh thành công! +{amt} xu 🪙",
        "Check-in berhasil! +{amt} koin 🪙", "Naka-check in! +{amt} coins 🪙", "เช็คอินแล้ว! +{amt} เหรียญ 🪙", "Ingecheckt! +{amt} munten 🪙", "Giriş yapıldı! +{amt} para 🪙",
        "Check-in feito! +{amt} moedas 🪙", "चेक-इन हो गया! +{amt} कॉइन 🪙", "تم التسجيل! +{amt} عملة 🪙")
    .replace("{amt}", amt)
}

pub fn checkin_already(l: Lang, time: &str) -> String {
    tr!(l;
        "Already checked in today — come back in {time} ⏳", "今天已經簽到了，{time} 後再來 ⏳", "今天已经签到了，{time} 后再来 ⏳", "今日はもうチェックイン済み。{time} 後にまた来てね ⏳", "오늘은 이미 출석했어 — {time} 후에 다시 와 ⏳",
        "Сегодня уже отмечались — возвращайтесь через {time} ⏳", "Déjà pointé aujourd'hui — reviens dans {time} ⏳", "Ya te registraste hoy — vuelve en {time} ⏳", "Heute schon eingecheckt — komm in {time} wieder ⏳", "Hôm nay đã điểm danh rồi — quay lại sau {time} ⏳",
        "Sudah check-in hari ini — kembali dalam {time} ⏳", "Naka-check in ka na ngayon — balik ka sa loob ng {time} ⏳", "วันนี้เช็คอินแล้ว — กลับมาในอีก {time} ⏳", "Vandaag al ingecheckt — kom over {time} terug ⏳", "Bugün giriş yapıldı — {time} sonra tekrar gel ⏳",
        "Você já fez check-in hoje — volte em {time} ⏳", "आज चेक-इन हो चुका — {time} बाद आएं ⏳", "سجّلت اليوم بالفعل — عُد بعد {time} ⏳")
    .replace("{time}", time)
}

// ----------------------------------------------------------------------------
// `/markets` brief
// ----------------------------------------------------------------------------

pub fn markets_title(l: Lang) -> &'static str {
    tr!(l;
        "🌍 Market Brief", "🌍 市場速報", "🌍 市场速报", "🌍 マーケット速報", "🌍 마켓 브리핑",
        "🌍 Сводка рынков", "🌍 Aperçu des marchés", "🌍 Resumen de mercados", "🌍 Markt-Überblick", "🌍 Tổng quan thị trường",
        "🌍 Ringkasan pasar", "🌍 Buod ng Market", "🌍 สรุปตลาด", "🌍 Marktoverzicht", "🌍 Piyasa Özeti",
        "🌍 Resumo do mercado", "🌍 मार्केट ब्रीफ", "🌍 موجز السوق")
}

pub fn markets_section(l: Lang) -> &'static str {
    tr!(l;
        "⚽ Markets:", "⚽ 市場：", "⚽ 市场：", "⚽ マーケット：", "⚽ 마켓:",
        "⚽ Рынки:", "⚽ Marchés :", "⚽ Mercados:", "⚽ Märkte:", "⚽ Thị trường:",
        "⚽ Pasar:", "⚽ Mga Market:", "⚽ ตลาด:", "⚽ Markten:", "⚽ Piyasalar:",
        "⚽ Mercados:", "⚽ मार्केट:", "⚽ الأسواق:")
}

pub fn markets_empty(l: Lang) -> &'static str {
    tr!(l;
        "No open markets right now 🪹", "目前沒有開放的市場 🪹", "目前没有开放的市场 🪹", "今は開いている市場がないよ 🪹", "지금은 열린 마켓이 없어 🪹",
        "Сейчас нет открытых рынков 🪹", "Aucun marché ouvert pour le moment 🪹", "No hay mercados abiertos ahora 🪹", "Derzeit keine offenen Märkte 🪹", "Hiện chưa có thị trường nào 🪹",
        "Belum ada pasar yang dibuka 🪹", "Walang bukas na market ngayon 🪹", "ตอนนี้ยังไม่มีตลาดที่เปิด 🪹", "Nu geen open markten 🪹", "Şu an açık piyasa yok 🪹",
        "Nenhum mercado aberto agora 🪹", "अभी कोई खुला मार्केट नहीं 🪹", "لا أسواق مفتوحة الآن 🪹")
}

pub fn markets_unavailable(l: Lang) -> &'static str {
    tr!(l;
        "Couldn't reach the markets 😵‍💫", "無法連線到市場 😵‍💫", "无法连接到市场 😵‍💫", "マーケットに接続できなかったよ 😵‍💫", "마켓에 연결하지 못했어 😵‍💫",
        "Не удалось получить рынки 😵‍💫", "Impossible de joindre les marchés 😵‍💫", "No se pudo acceder a los mercados 😵‍💫", "Märkte nicht erreichbar 😵‍💫", "Không kết nối được tới thị trường 😵‍💫",
        "Tidak bisa mengakses pasar 😵‍💫", "Hindi maabot ang market 😵‍💫", "เชื่อมต่อตลาดไม่ได้ 😵‍💫", "Kon de markten niet bereiken 😵‍💫", "Piyasalara ulaşılamadı 😵‍💫",
        "Não foi possível acessar os mercados 😵‍💫", "मार्केट तक नहीं पहुँच सके 😵‍💫", "تعذّر الوصول إلى الأسواق 😵‍💫")
}

/// Page indicator shown when the brief spans more than one page.
pub fn markets_page(l: Lang, cur: &str, total: &str) -> String {
    tr!(l;
        "📄 Page {cur}/{total}", "📄 第 {cur}/{total} 頁", "📄 第 {cur}/{total} 页", "📄 {cur}/{total} ページ", "📄 {cur}/{total} 페이지",
        "📄 Стр. {cur}/{total}", "📄 Page {cur}/{total}", "📄 Página {cur}/{total}", "📄 Seite {cur}/{total}", "📄 Trang {cur}/{total}",
        "📄 Halaman {cur}/{total}", "📄 Pahina {cur}/{total}", "📄 หน้า {cur}/{total}", "📄 Pagina {cur}/{total}", "📄 Sayfa {cur}/{total}",
        "📄 Página {cur}/{total}", "📄 पृष्ठ {cur}/{total}", "📄 صفحة {cur}/{total}")
    .replace("{cur}", cur)
    .replace("{total}", total)
}

/// "Next page" navigation button.
pub fn markets_next(l: Lang) -> &'static str {
    tr!(l;
        "Next ▸", "下一頁 ▸", "下一页 ▸", "次へ ▸", "다음 ▸",
        "Дальше ▸", "Suivant ▸", "Siguiente ▸", "Weiter ▸", "Tiếp ▸",
        "Berikutnya ▸", "Susunod ▸", "ถัดไป ▸", "Volgende ▸", "İleri ▸",
        "Próximo ▸", "आगे ▸", "▸ التالي")
}

/// "Previous page" navigation button.
pub fn markets_prev(l: Lang) -> &'static str {
    tr!(l;
        "◂ Prev", "◂ 上一頁", "◂ 上一页", "◂ 前へ", "◂ 이전",
        "◂ Назад", "◂ Précédent", "◂ Anterior", "◂ Zurück", "◂ Trước",
        "◂ Sebelumnya", "◂ Nakaraan", "◂ ก่อนหน้า", "◂ Vorige", "◂ Geri",
        "◂ Anterior", "◂ पिछला", "السابق ◂")
}

// ----------------------------------------------------------------------------
// Market betting
// ----------------------------------------------------------------------------

pub fn bet_pick(l: Lang) -> &'static str {
    tr!(l;
        "Pick your side:", "選擇你的下注：", "选择你的下注：", "賭ける側を選んで：", "베팅할 쪽을 선택:",
        "Выберите сторону:", "Choisis ton camp :", "Elige tu lado:", "Wähle deine Seite:", "Chọn bên cược:",
        "Pilih sisi taruhanmu:", "Piliin ang panig:", "เลือกฝั่งที่จะเดิมพัน:", "Kies je kant:", "Tarafını seç:",
        "Escolha seu lado:", "अपना पक्ष चुनें:", "اختر جانبك:")
}

pub fn bet_unavailable(l: Lang) -> &'static str {
    tr!(l;
        "Couldn't load this market — open /markets again 😶", "無法載入這個市場，請重新開啟 /markets 😶", "无法加载这个市场，请重新打开 /markets 😶", "この市場を読み込めなかった。/markets を開き直してね 😶", "이 마켓을 못 불러왔어. /markets 를 다시 열어줘 😶",
        "Не удалось загрузить рынок — открой /markets заново 😶", "Impossible de charger ce marché — rouvre /markets 😶", "No se pudo cargar este mercado — abre /markets otra vez 😶", "Markt konnte nicht geladen werden — /markets erneut öffnen 😶", "Không tải được thị trường này — mở lại /markets 😶",
        "Gagal memuat pasar ini — buka /markets lagi 😶", "Hindi ma-load ang market — buksan ulit ang /markets 😶", "โหลดตลาดนี้ไม่ได้ — เปิด /markets อีกครั้ง 😶", "Kon deze markt niet laden — open /markets opnieuw 😶", "Bu piyasa yüklenemedi — /markets'i tekrar aç 😶",
        "Não foi possível carregar este mercado — abra /markets de novo 😶", "यह मार्केट लोड नहीं हो सका — /markets फिर खोलें 😶", "تعذّر تحميل هذا السوق — افتح /markets من جديد 😶")
}

pub fn bet_closed(l: Lang) -> &'static str {
    tr!(l;
        "Betting for this market has closed ⏱️", "這個市場已停止下注 ⏱️", "这个市场已停止下注 ⏱️", "この市場の賭けは締め切られたよ ⏱️", "이 마켓 베팅은 마감됐어 ⏱️",
        "Ставки на этот рынок закрыты ⏱️", "Les paris sur ce marché sont clos ⏱️", "Las apuestas para este mercado están cerradas ⏱️", "Wetten für diesen Markt sind geschlossen ⏱️", "Đã đóng cược cho thị trường này ⏱️",
        "Taruhan untuk pasar ini sudah ditutup ⏱️", "Sarado na ang pagtaya para sa market na ito ⏱️", "ปิดรับเดิมพันตลาดนี้แล้ว ⏱️", "Wedden op deze markt is gesloten ⏱️", "Bu piyasa için bahisler kapandı ⏱️",
        "As apostas para este mercado estão encerradas ⏱️", "इस मार्केट पर बेटिंग बंद हो चुकी है ⏱️", "أُغلق الرهان على هذا السوق ⏱️")
}

pub fn market_finished(l: Lang) -> &'static str {
    tr!(l;
        "🏁 This market has finished — bets will be settled soon.", "🏁 這個市場已結束，賭注將盡快結算。", "🏁 这个市场已结束，赌注将尽快结算。", "🏁 この市場は終了したよ。賭けはまもなく精算されるよ。", "🏁 이 마켓은 끝났어. 베팅은 곧 정산될 거야.",
        "🏁 Рынок завершён — ставки скоро рассчитают.", "🏁 Ce marché est terminé — les paris seront réglés bientôt.", "🏁 Este mercado ha terminado — las apuestas se liquidarán pronto.", "🏁 Dieser Markt ist beendet — Wetten werden bald abgerechnet.", "🏁 Thị trường này đã kết thúc — cược sẽ sớm được quyết toán.",
        "🏁 Pasar ini sudah selesai — taruhan akan segera diselesaikan.", "🏁 Tapos na ang market na ito — aayusin ang mga pusta sa lalong madaling panahon.", "🏁 ตลาดนี้จบแล้ว — จะชำระเงินเดิมพันเร็ว ๆ นี้", "🏁 Deze markt is afgelopen — weddenschappen worden binnenkort afgewikkeld.", "🏁 Bu piyasa sona erdi — bahisler yakında sonuçlanacak.",
        "🏁 Este mercado terminou — as apostas serão liquidadas em breve.", "🏁 यह मार्केट समाप्त हो गया — दांव जल्द ही निपटाए जाएंगे।", "🏁 انتهى هذا السوق — ستتم تسوية الرهانات قريبًا.")
}

pub fn bet_dm_first(l: Lang) -> &'static str {
    tr!(l;
        "Start a private chat with me first to place bets 📩", "請先私訊我才能下注 📩", "请先私信我才能下注 📩", "賭けるにはまず私にDMしてね 📩", "베팅하려면 먼저 나에게 DM을 보내줘 📩",
        "Сначала напишите мне в личку, чтобы делать ставки 📩", "Écris-moi d'abord en privé pour parier 📩", "Primero abre un chat privado conmigo para apostar 📩", "Schreib mir zuerst privat, um zu wetten 📩", "Hãy nhắn riêng cho mình trước để đặt cược 📩",
        "Mulai chat pribadi dulu untuk bertaruh 📩", "Mag-DM muna sa akin para makataya 📩", "เริ่มแชทส่วนตัวกับฉันก่อนเพื่อเดิมพัน 📩", "Begin eerst een privégesprek met mij om te wedden 📩", "Bahis için önce bana özelden yaz 📩",
        "Abra um chat privado comigo primeiro para apostar 📩", "दांव लगाने के लिए पहले मुझसे निजी चैट शुरू करें 📩", "ابدأ محادثة خاصة معي أولًا للمراهنة 📩")
}

pub fn bet_expired(l: Lang) -> &'static str {
    tr!(l;
        "⌛ Odds changed — open /markets again.", "⌛ 賠率已變動，請重新開啟 /markets。", "⌛ 赔率已变动，请重新打开 /markets。", "⌛ オッズが変わったよ。/markets をもう一度開いてね。", "⌛ 배당이 변경됐어 — /markets 를 다시 열어줘.",
        "⌛ Коэффициенты изменились — откройте /markets снова.", "⌛ Les cotes ont changé — rouvre /markets.", "⌛ Las cuotas cambiaron — abre /markets de nuevo.", "⌛ Quoten geändert — öffne /markets erneut.", "⌛ Tỷ lệ đã thay đổi — mở lại /markets.",
        "⌛ Odds berubah — buka /markets lagi.", "⌛ Nagbago ang odds — buksan ulit ang /markets.", "⌛ อัตราต่อรองเปลี่ยนแล้ว — เปิด /markets อีกครั้ง", "⌛ Odds gewijzigd — open /markets opnieuw.", "⌛ Oranlar değişti — /markets'i tekrar aç.",
        "⌛ As odds mudaram — abra /markets de novo.", "⌛ ऑड्स बदल गए — /markets फिर से खोलें।", "⌛ تغيّرت الاحتمالات — افتح /markets من جديد.")
}

pub fn bet_done(l: Lang) -> &'static str {
    tr!(l;
        "Bet placed ✅", "已下注 ✅", "已下注 ✅", "賭け完了 ✅", "베팅 완료 ✅",
        "Ставка принята ✅", "Pari placé ✅", "Apuesta hecha ✅", "Wette platziert ✅", "Đã đặt cược ✅",
        "Taruhan dipasang ✅", "Tumaya na ✅", "วางเดิมพันแล้ว ✅", "Inzet geplaatst ✅", "Bahis kondu ✅",
        "Aposta feita ✅", "दांव लगा ✅", "تم وضع الرهان ✅")
}

pub fn bet_lost(l: Lang, m: &str) -> String {
    tr!(l;
        "😔 {market}\nYour bet didn't win this time.", "😔 {market}\n這次下注沒中。", "😔 {market}\n这次下注没中。", "😔 {market}\n今回は外れたよ。", "😔 {market}\n이번 베팅은 졌어.",
        "😔 {market}\nВ этот раз ставка не сыграла.", "😔 {market}\nTon pari n'a pas gagné cette fois.", "😔 {market}\nTu apuesta no ganó esta vez.", "😔 {market}\nDeine Wette hat diesmal nicht gewonnen.", "😔 {market}\nLần này cược của bạn không thắng.",
        "😔 {market}\nTaruhanmu kali ini kalah.", "😔 {market}\nHindi nanalo ang taya mo ngayon.", "😔 {market}\nครั้งนี้เดิมพันไม่ชนะ", "😔 {market}\nJe weddenschap heeft deze keer niet gewonnen.", "😔 {market}\nBahsin bu sefer kazanmadı.",
        "😔 {market}\nSua aposta não ganhou desta vez.", "😔 {market}\nइस बार आपका दांव नहीं जीता।", "😔 {market}\nلم يفز رهانك هذه المرة.")
    .replace("{market}", m)
}

/// Stake-builder screen: the chosen side/odds, the running stake and potential
/// win, and a prompt to tap presets to add.
pub fn bet_build(l: Lang, side: &str, odds: &str, stake: &str, win: &str) -> String {
    tr!(l;
        "{side} @ {odds}\nStake: {stake} 🪙 → win {win} 🪙\nTap to add:", "{side} @ {odds}\n下注：{stake} 🪙 → 贏 {win} 🪙\n點擊加注：", "{side} @ {odds}\n下注：{stake} 🪙 → 赢 {win} 🪙\n点击加注：", "{side} @ {odds}\nステーク：{stake} 🪙 → 勝てば {win} 🪙\nタップで追加：", "{side} @ {odds}\n베팅: {stake} 🪙 → 당첨 시 {win} 🪙\n탭하여 추가:",
        "{side} @ {odds}\nСтавка: {stake} 🪙 → выигрыш {win} 🪙\nНажмите, чтобы добавить:", "{side} @ {odds}\nMise : {stake} 🪙 → gain {win} 🪙\nAppuyez pour ajouter :", "{side} @ {odds}\nApuesta: {stake} 🪙 → ganas {win} 🪙\nToca para añadir:", "{side} @ {odds}\nEinsatz: {stake} 🪙 → Gewinn {win} 🪙\nZum Hinzufügen tippen:", "{side} @ {odds}\nĐặt: {stake} 🪙 → thắng {win} 🪙\nChạm để thêm:",
        "{side} @ {odds}\nTaruhan: {stake} 🪙 → menang {win} 🪙\nKetuk untuk menambah:", "{side} @ {odds}\nTaya: {stake} 🪙 → panalo {win} 🪙\nI-tap para magdagdag:", "{side} @ {odds}\nเดิมพัน: {stake} 🪙 → ชนะ {win} 🪙\nแตะเพื่อเพิ่ม:", "{side} @ {odds}\nInzet: {stake} 🪙 → winst {win} 🪙\nTik om toe te voegen:", "{side} @ {odds}\nBahis: {stake} 🪙 → kazanç {win} 🪙\nEklemek için dokun:",
        "{side} @ {odds}\nAposta: {stake} 🪙 → ganha {win} 🪙\nToque para adicionar:", "{side} @ {odds}\nदांव: {stake} 🪙 → जीत {win} 🪙\nजोड़ने के लिए टैप करें:", "{side} @ {odds}\nالرهان: {stake} 🪙 → الربح {win} 🪙\nانقر للإضافة:")
    .replace("{side}", side)
    .replace("{odds}", odds)
    .replace("{stake}", stake)
    .replace("{win}", win)
}

/// Final confirmation screen before debiting the balance.
pub fn bet_confirm(l: Lang, stake: &str, side: &str, win: &str) -> String {
    tr!(l;
        "Place {stake} 🪙 on {side}?\nPotential win: {win} 🪙", "確定下注 {stake} 🪙 於 {side}？\n潛在贏得：{win} 🪙", "确定下注 {stake} 🪙 于 {side}？\n潜在赢得：{win} 🪙", "{side} に {stake} 🪙 を賭けますか？\n勝てば：{win} 🪙", "{side} 에 {stake} 🪙 베팅할까요?\n당첨 시: {win} 🪙",
        "Поставить {stake} 🪙 на {side}?\nВозможный выигрыш: {win} 🪙", "Miser {stake} 🪙 sur {side} ?\nGain potentiel : {win} 🪙", "¿Apostar {stake} 🪙 a {side}?\nGanancia potencial: {win} 🪙", "{stake} 🪙 auf {side} setzen?\nMöglicher Gewinn: {win} 🪙", "Đặt {stake} 🪙 cho {side}?\nTiền thắng dự kiến: {win} 🪙",
        "Pasang {stake} 🪙 pada {side}?\nPotensi menang: {win} 🪙", "Itaya ang {stake} 🪙 sa {side}?\nPosibleng panalo: {win} 🪙", "วางเดิมพัน {stake} 🪙 ที่ {side}?\nเงินรางวัลที่อาจได้: {win} 🪙", "{stake} 🪙 op {side} inzetten?\nMogelijke winst: {win} 🪙", "{side} için {stake} 🪙 yatırılsın mı?\nOlası kazanç: {win} 🪙",
        "Apostar {stake} 🪙 em {side}?\nGanho potencial: {win} 🪙", "{side} पर {stake} 🪙 लगाएं?\nसंभावित जीत: {win} 🪙", "وضع {stake} 🪙 على {side}؟\nالربح المحتمل: {win} 🪙")
    .replace("{stake}", stake)
    .replace("{side}", side)
    .replace("{win}", win)
}

pub fn bet_btn_confirm(l: Lang) -> &'static str {
    tr!(l;
        "✅ Confirm", "✅ 確認", "✅ 确认", "✅ 確認", "✅ 확인",
        "✅ Подтвердить", "✅ Confirmer", "✅ Confirmar", "✅ Bestätigen", "✅ Xác nhận",
        "✅ Konfirmasi", "✅ Kumpirmahin", "✅ ยืนยัน", "✅ Bevestigen", "✅ Onayla",
        "✅ Confirmar", "✅ पुष्टि करें", "✅ تأكيد")
}

pub fn bet_btn_clear(l: Lang) -> &'static str {
    tr!(l;
        "🗑 Clear", "🗑 清除", "🗑 清除", "🗑 クリア", "🗑 초기화",
        "🗑 Сбросить", "🗑 Effacer", "🗑 Borrar", "🗑 Löschen", "🗑 Xóa",
        "🗑 Hapus", "🗑 Burahin", "🗑 ล้าง", "🗑 Wissen", "🗑 Temizle",
        "🗑 Limpar", "🗑 साफ़ करें", "🗑 مسح")
}

pub fn bet_btn_place(l: Lang) -> &'static str {
    tr!(l;
        "✅ Place bet", "✅ 確定下注", "✅ 确定下注", "✅ 賭ける", "✅ 베팅하기",
        "✅ Сделать ставку", "✅ Parier", "✅ Apostar", "✅ Wetten", "✅ Đặt cược",
        "✅ Pasang", "✅ Itaya", "✅ วางเดิมพัน", "✅ Inzetten", "✅ Bahis yap",
        "✅ Apostar", "✅ दांव लगाएं", "✅ ضع الرهان")
}

pub fn bet_btn_back(l: Lang) -> &'static str {
    tr!(l;
        "⬅ Back", "⬅ 返回", "⬅ 返回", "⬅ 戻る", "⬅ 뒤로",
        "⬅ Назад", "⬅ Retour", "⬅ Atrás", "⬅ Zurück", "⬅ Quay lại",
        "⬅ Kembali", "⬅ Bumalik", "⬅ ย้อนกลับ", "⬅ Terug", "⬅ Geri",
        "⬅ Voltar", "⬅ वापस", "⬅ رجوع")
}

/// Dismiss button on an in-group personal bet board — deletes the board.
pub fn bet_btn_dismiss(l: Lang) -> &'static str {
    tr!(l;
        "✖ Dismiss", "✖ 取消", "✖ 取消", "✖ 取り消し", "✖ 닫기",
        "✖ Закрыть", "✖ Annuler", "✖ Descartar", "✖ Verwerfen", "✖ Bỏ",
        "✖ Tutup", "✖ Isara", "✖ ปิด", "✖ Sluiten", "✖ Kapat",
        "✖ Descartar", "✖ हटाएं", "✖ إغلاق")
}

/// Toast when someone taps a personal bet board that isn't theirs (owner-locked).
pub fn not_your_bet(l: Lang) -> &'static str {
    tr!(l;
        "This isn't your bet 🙅", "這不是你的下注 🙅", "这不是你的下注 🙅", "これはあなたの賭けじゃないよ 🙅", "이건 네 베팅이 아니야 🙅",
        "Это не твоя ставка 🙅", "Ce n'est pas ton pari 🙅", "Esta no es tu apuesta 🙅", "Das ist nicht deine Wette 🙅", "Đây không phải cược của bạn 🙅",
        "Ini bukan taruhanmu 🙅", "Hindi ito ang taya mo 🙅", "นี่ไม่ใช่เดิมพันของคุณ 🙅", "Dit is niet jouw weddenschap 🙅", "Bu senin bahsin değil 🙅",
        "Esta não é a sua aposta 🙅", "यह आपका दांव नहीं है 🙅", "هذا ليس رهانك 🙅")
}

pub fn bet_placed(l: Lang, stake: &str, side: &str, odds: &str, payout: &str) -> String {
    tr!(l;
        "✅ Bet placed: {stake} on {side} @ {odds}\nPotential payout: {payout}", "✅ 已下注：{stake} 於 {side} @ {odds}\n潛在派彩：{payout}", "✅ 已下注：{stake} 于 {side} @ {odds}\n潜在派彩：{payout}", "✅ 賭け完了：{side} @ {odds} に {stake}\n払い戻し見込み：{payout}", "✅ 베팅 완료: {side} @ {odds} 에 {stake}\n예상 수령: {payout}",
        "✅ Ставка принята: {stake} на {side} @ {odds}\nВозможный выигрыш: {payout}", "✅ Pari placé : {stake} sur {side} @ {odds}\nGain potentiel : {payout}", "✅ Apuesta hecha: {stake} a {side} @ {odds}\nPago potencial: {payout}", "✅ Wette platziert: {stake} auf {side} @ {odds}\nMöglicher Gewinn: {payout}", "✅ Đã đặt cược: {stake} cho {side} @ {odds}\nTiền thắng dự kiến: {payout}",
        "✅ Taruhan dipasang: {stake} pada {side} @ {odds}\nPotensi bayaran: {payout}", "✅ Tumaya: {stake} sa {side} @ {odds}\nPosibleng panalo: {payout}", "✅ วางเดิมพันแล้ว: {stake} ที่ {side} @ {odds}\nเงินรางวัลที่อาจได้: {payout}", "✅ Inzet geplaatst: {stake} op {side} @ {odds}\nMogelijke uitbetaling: {payout}", "✅ Bahis kondu: {side} @ {odds} için {stake}\nOlası kazanç: {payout}",
        "✅ Aposta feita: {stake} em {side} @ {odds}\nPagamento potencial: {payout}", "✅ दांव लगा: {side} @ {odds} पर {stake}\nसंभावित भुगतान: {payout}", "✅ تم وضع الرهان: {stake} على {side} @ {odds}\nالعائد المحتمل: {payout}")
    .replace("{stake}", stake)
    .replace("{side}", side)
    .replace("{odds}", odds)
    .replace("{payout}", payout)
}

/// Third-person announcement of a placed bet, posted back to the origin group.
pub fn bet_announce(l: Lang, name: &str, stake: &str, side: &str, odds: &str) -> String {
    tr!(l;
        "🎟️ {name} bet {stake} 🪙 on {side} @ {odds}", "🎟️ {name} 下注 {stake} 🪙 於 {side} @ {odds}", "🎟️ {name} 下注 {stake} 🪙 于 {side} @ {odds}", "🎟️ {name} が {side} @ {odds} に {stake} 🪙 を賭けた", "🎟️ {name} 님이 {side} @ {odds} 에 {stake} 🪙 베팅",
        "🎟️ {name} поставил {stake} 🪙 на {side} @ {odds}", "🎟️ {name} a misé {stake} 🪙 sur {side} @ {odds}", "🎟️ {name} apostó {stake} 🪙 a {side} @ {odds}", "🎟️ {name} hat {stake} 🪙 auf {side} @ {odds} gesetzt", "🎟️ {name} đã đặt {stake} 🪙 cho {side} @ {odds}",
        "🎟️ {name} bertaruh {stake} 🪙 pada {side} @ {odds}", "🎟️ {name} tumaya ng {stake} 🪙 sa {side} @ {odds}", "🎟️ {name} เดิมพัน {stake} 🪙 ที่ {side} @ {odds}", "🎟️ {name} zette {stake} 🪙 op {side} @ {odds}", "🎟️ {name}, {side} @ {odds} için {stake} 🪙 yatırdı",
        "🎟️ {name} apostou {stake} 🪙 em {side} @ {odds}", "🎟️ {name} ने {side} @ {odds} पर {stake} 🪙 लगाए", "🎟️ {name} راهن {stake} 🪙 على {side} @ {odds}")
    .replace("{name}", name)
    .replace("{stake}", stake)
    .replace("{side}", side)
    .replace("{odds}", odds)
}

pub fn btn_sell(l: Lang) -> &'static str {
    tr!(l;
        "💸 Sell", "💸 出售", "💸 出售", "💸 売却", "💸 판매",
        "💸 Продать", "💸 Vendre", "💸 Vender", "💸 Verkaufen", "💸 Bán",
        "💸 Jual", "💸 Ibenta", "💸 ขาย", "💸 Verkopen", "💸 Sat",
        "💸 Vender", "💸 बेचें", "💸 بيع")
}

pub fn sell_build(l: Lang, outcome: &str, held: &str, selling: &str, proceeds: &str) -> String {
    tr!(l;
        "💸 Sell {outcome}\nHeld {held} · selling {selling} → ≈ {proceeds} 🪙\nChoose amount:", "💸 出售 {outcome}\n持有 {held} · 賣出 {selling} → ≈ {proceeds} 🪙\n選擇數量：", "💸 出售 {outcome}\n持有 {held} · 卖出 {selling} → ≈ {proceeds} 🪙\n选择数量：", "💸 {outcome} を売却\n保有 {held} · 売却 {selling} → ≈ {proceeds} 🪙\n数量を選択：", "💸 {outcome} 판매\n보유 {held} · 판매 {selling} → ≈ {proceeds} 🪙\n수량 선택:",
        "💸 Продать {outcome}\nЕсть {held} · продаём {selling} → ≈ {proceeds} 🪙\nВыберите количество:", "💸 Vendre {outcome}\nDétenu {held} · vente {selling} → ≈ {proceeds} 🪙\nChoisissez le montant :", "💸 Vender {outcome}\nTienes {held} · vendiendo {selling} → ≈ {proceeds} 🪙\nElige la cantidad:", "💸 {outcome} verkaufen\nBestand {held} · Verkauf {selling} → ≈ {proceeds} 🪙\nMenge wählen:", "💸 Bán {outcome}\nĐang giữ {held} · bán {selling} → ≈ {proceeds} 🪙\nChọn số lượng:",
        "💸 Jual {outcome}\nDimiliki {held} · menjual {selling} → ≈ {proceeds} 🪙\nPilih jumlah:", "💸 Ibenta ang {outcome}\nHawak {held} · ibinebenta {selling} → ≈ {proceeds} 🪙\nPumili ng halaga:", "💸 ขาย {outcome}\nถือ {held} · ขาย {selling} → ≈ {proceeds} 🪙\nเลือกจำนวน:", "💸 {outcome} verkopen\nIn bezit {held} · verkoop {selling} → ≈ {proceeds} 🪙\nKies hoeveelheid:", "💸 {outcome} sat\nElde {held} · satılıyor {selling} → ≈ {proceeds} 🪙\nMiktar seç:",
        "💸 Vender {outcome}\nEm carteira {held} · vendendo {selling} → ≈ {proceeds} 🪙\nEscolha a quantidade:", "💸 {outcome} बेचें\nधारित {held} · बेच रहे {selling} → ≈ {proceeds} 🪙\nमात्रा चुनें:", "💸 بيع {outcome}\nلديك {held} · تبيع {selling} → ≈ {proceeds} 🪙\nاختر الكمية:")
    .replace("{outcome}", outcome)
    .replace("{held}", held)
    .replace("{selling}", selling)
    .replace("{proceeds}", proceeds)
}

pub fn sold(l: Lang, shares: &str, outcome: &str, proceeds: &str) -> String {
    tr!(l;
        "✅ Sold {shares} {outcome} for {proceeds} 🪙", "✅ 已賣出 {shares} {outcome}，得 {proceeds} 🪙", "✅ 已卖出 {shares} {outcome}，得 {proceeds} 🪙", "✅ {outcome} を {shares} 売却し {proceeds} 🪙 を獲得", "✅ {outcome} {shares} 판매, {proceeds} 🪙 획득",
        "✅ Продано {shares} {outcome} за {proceeds} 🪙", "✅ Vendu {shares} {outcome} pour {proceeds} 🪙", "✅ Vendiste {shares} {outcome} por {proceeds} 🪙", "✅ {shares} {outcome} für {proceeds} 🪙 verkauft", "✅ Đã bán {shares} {outcome} được {proceeds} 🪙",
        "✅ Terjual {shares} {outcome} seharga {proceeds} 🪙", "✅ Naibenta {shares} {outcome} sa {proceeds} 🪙", "✅ ขาย {shares} {outcome} ได้ {proceeds} 🪙", "✅ {shares} {outcome} verkocht voor {proceeds} 🪙", "✅ {shares} {outcome} {proceeds} 🪙 karşılığında satıldı",
        "✅ Vendeu {shares} {outcome} por {proceeds} 🪙", "✅ {shares} {outcome} {proceeds} 🪙 में बेचा", "✅ تم بيع {shares} {outcome} مقابل {proceeds} 🪙")
    .replace("{shares}", shares)
    .replace("{outcome}", outcome)
    .replace("{proceeds}", proceeds)
}

/// Realized profit/loss line appended to the sell confirmation. `pnl` is a
/// pre-signed amount (e.g. `+5` / `-3`); the emoji conveys the direction.
pub fn sell_pnl(l: Lang, emoji: &str, pnl: &str) -> String {
    tr!(l;
        "{emoji} P&L: {pnl} 🪙", "{emoji} 盈虧：{pnl} 🪙", "{emoji} 盈亏：{pnl} 🪙", "{emoji} 損益：{pnl} 🪙", "{emoji} 손익: {pnl} 🪙",
        "{emoji} Прибыль/убыток: {pnl} 🪙", "{emoji} P&L : {pnl} 🪙", "{emoji} P&L: {pnl} 🪙", "{emoji} G&V: {pnl} 🪙", "{emoji} Lãi/lỗ: {pnl} 🪙",
        "{emoji} L/R: {pnl} 🪙", "{emoji} Tubo/Lugi: {pnl} 🪙", "{emoji} กำไร/ขาดทุน: {pnl} 🪙", "{emoji} W/V: {pnl} 🪙", "{emoji} K/Z: {pnl} 🪙",
        "{emoji} L&P: {pnl} 🪙", "{emoji} लाभ/हानि: {pnl} 🪙", "{emoji} الربح/الخسارة: {pnl} 🪙")
    .replace("{emoji}", emoji)
    .replace("{pnl}", pnl)
}

pub fn predict_ask_fee(l: Lang) -> &'static str {
    tr!(l;
        "💸 Pick your trading fee — you earn it on every trade (max 10%):", "💸 選擇你的交易手續費 — 每筆交易你都能賺取（上限 10%）：", "💸 选择你的交易手续费 — 每笔交易你都能赚取（上限 10%）：", "💸 取引手数料を選択 — すべての取引で獲得できます（最大10%）：", "💸 거래 수수료 선택 — 모든 거래에서 받습니다 (최대 10%):",
        "💸 Выберите торговую комиссию — вы получаете её с каждой сделки (макс. 10%):", "💸 Choisissez vos frais — vous les gagnez sur chaque échange (max 10 %) :", "💸 Elige tu comisión — la ganas en cada operación (máx. 10%):", "💸 Wähle deine Handelsgebühr — du verdienst sie bei jedem Trade (max. 10%):", "💸 Chọn phí giao dịch — bạn kiếm được phí này mỗi giao dịch (tối đa 10%):",
        "💸 Pilih biaya transaksi — Anda mendapatkannya tiap transaksi (maks 10%):", "💸 Piliin ang iyong bayarin — kikita mo ito sa bawat transaksyon (max 10%):", "💸 เลือกค่าธรรมเนียม — คุณได้รับทุกการเทรด (สูงสุด 10%):", "💸 Kies je handelskosten — je verdient ze bij elke trade (max 10%):", "💸 İşlem ücretini seç — her işlemde kazanırsın (en fazla %10):",
        "💸 Escolha sua taxa — você ganha em cada negociação (máx. 10%):", "💸 अपना ट्रेडिंग शुल्क चुनें — हर ट्रेड पर आप कमाते हैं (अधिकतम 10%):", "💸 اختر رسوم التداول — تكسبها مع كل صفقة (بحد أقصى 10%):")
}

pub fn predict_ask_funding(l: Lang) -> &'static str {
    tr!(l;
        "🌱 How long should the funding window stay open? LPs seed the pool and set the opening odds.", "🌱 資金注入視窗要開多久？造市者出資並設定開盤賠率。", "🌱 资金注入窗口开多久？做市商出资并设定开盘赔率。", "🌱 資金提供の受付時間は？LPが資金を入れ、開始オッズを決めます。", "🌱 펀딩 창을 얼마나 열까요? LP가 풀을 채우고 시작 배당을 정합니다.",
        "🌱 Сколько держать окно фондирования? LP наполняют пул и задают стартовые шансы.", "🌱 Combien de temps ouvrir le financement ? Les LP alimentent le pool et fixent la cote d'ouverture.", "🌱 ¿Cuánto dura la ventana de financiación? Los LP siembran el pool y fijan la cuota inicial.", "🌱 Wie lange soll das Funding-Fenster offen sein? LPs füllen den Pool und setzen die Eröffnungsquote.", "🌱 Cửa sổ góp vốn mở bao lâu? LP góp vốn và đặt tỷ lệ mở cửa.",
        "🌱 Berapa lama jendela pendanaan dibuka? LP mengisi pool dan menetapkan odds awal.", "🌱 Gaano katagal ang funding window? Pinopondohan ng LP ang pool at itinatakda ang opening odds.", "🌱 เปิดช่วงระดมทุนนานแค่ไหน? LP เติมพูลและตั้งราคาเปิด", "🌱 Hoe lang blijft het funding-venster open? LP's vullen de pool en zetten de openingsodds.", "🌱 Fonlama penceresi ne kadar açık kalsın? LP'ler havuzu doldurur ve açılış oranını belirler.",
        "🌱 Por quanto tempo a janela de financiamento fica aberta? Os LPs abastecem o pool e definem as odds de abertura.", "🌱 फंडिंग विंडो कितनी देर खुली रहे? LP पूल भरते हैं और शुरुआती ऑड्स तय करते हैं।", "🌱 كم تبقى نافذة التمويل مفتوحة؟ يموّل مزودو السيولة المجمع ويحددون الاحتمالات الافتتاحية.")
}

pub fn fund_build(l: Lang, name: &str, amount: &str, price: &str) -> String {
    tr!(l;
        "💧 Fund {name}\nAdd {amount} 🪙 · opens → {price}\nTap to add:", "💧 注資 {name}\n加 {amount} 🪙 · 開盤 → {price}\n點擊加注：", "💧 注资 {name}\n加 {amount} 🪙 · 开盘 → {price}\n点击加注：", "💧 {name} に出資\n{amount} 🪙 追加 · 開始 → {price}\nタップで追加：", "💧 {name} 펀딩\n{amount} 🪙 추가 · 시작 → {price}\n탭하여 추가:",
        "💧 Фондировать {name}\n+{amount} 🪙 · открытие → {price}\nНажмите, чтобы добавить:", "💧 Financer {name}\n+{amount} 🪙 · ouverture → {price}\nAppuyez pour ajouter :", "💧 Financiar {name}\n+{amount} 🪙 · apertura → {price}\nToca para añadir:", "💧 {name} finanzieren\n+{amount} 🪙 · Eröffnung → {price}\nZum Hinzufügen tippen:", "💧 Góp vốn {name}\n+{amount} 🪙 · mở → {price}\nChạm để thêm:",
        "💧 Danai {name}\n+{amount} 🪙 · pembukaan → {price}\nKetuk untuk menambah:", "💧 Pondohan ang {name}\n+{amount} 🪙 · pagbubukas → {price}\nI-tap para magdagdag:", "💧 เติมทุน {name}\n+{amount} 🪙 · เปิด → {price}\nแตะเพื่อเพิ่ม:", "💧 Financier {name}\n+{amount} 🪙 · opening → {price}\nTik om toe te voegen:", "💧 {name} fonla\n+{amount} 🪙 · açılış → {price}\nEklemek için dokun:",
        "💧 Financiar {name}\n+{amount} 🪙 · abertura → {price}\nToque para adicionar:", "💧 {name} फंड करें\n+{amount} 🪙 · ओपनिंग → {price}\nजोड़ने के लिए टैप करें:", "💧 تمويل {name}\n+{amount} 🪙 · الافتتاح → {price}\nانقر للإضافة:")
    .replace("{name}", name)
    .replace("{amount}", amount)
    .replace("{price}", price)
}

pub fn fund_done(l: Lang, name: &str, amount: &str) -> String {
    tr!(l;
        "💧 Funded {name} with {amount} 🪙", "💧 已為 {name} 注資 {amount} 🪙", "💧 已为 {name} 注资 {amount} 🪙", "💧 {name} に {amount} 🪙 出資しました", "💧 {name}에 {amount} 🪙 펀딩 완료",
        "💧 Профондировано {name} на {amount} 🪙", "💧 {name} financé de {amount} 🪙", "💧 {name} financiado con {amount} 🪙", "💧 {name} mit {amount} 🪙 finanziert", "💧 Đã góp {amount} 🪙 cho {name}",
        "💧 Mendanai {name} dengan {amount} 🪙", "💧 Napondohan ang {name} ng {amount} 🪙", "💧 เติมทุน {name} {amount} 🪙 แล้ว", "💧 {name} gefinancierd met {amount} 🪙", "💧 {name} {amount} 🪙 ile fonlandı",
        "💧 {name} financiado com {amount} 🪙", "💧 {name} को {amount} 🪙 से फंड किया", "💧 تم تمويل {name} بمبلغ {amount} 🪙")
    .replace("{name}", name)
    .replace("{amount}", amount)
}

pub fn fund_announce(l: Lang, who: &str, amount: &str, name: &str) -> String {
    tr!(l;
        "💧 {who} seeded {amount} 🪙 on {name}", "💧 {who} 為 {name} 注資 {amount} 🪙", "💧 {who} 为 {name} 注资 {amount} 🪙", "💧 {who} が {name} に {amount} 🪙 出資", "💧 {who}님이 {name}에 {amount} 🪙 펀딩",
        "💧 {who} вложил {amount} 🪙 в {name}", "💧 {who} a misé {amount} 🪙 sur {name}", "💧 {who} aportó {amount} 🪙 a {name}", "💧 {who} hat {amount} 🪙 in {name} gesetzt", "💧 {who} đã góp {amount} 🪙 vào {name}",
        "💧 {who} mendanai {amount} 🪙 ke {name}", "💧 Pinondohan ni {who} ang {name} ng {amount} 🪙", "💧 {who} เติม {amount} 🪙 ให้ {name}", "💧 {who} zette {amount} 🪙 op {name}", "💧 {who}, {name} için {amount} 🪙 koydu",
        "💧 {who} aportou {amount} 🪙 em {name}", "💧 {who} ने {name} पर {amount} 🪙 लगाए", "💧 {who} موّل {name} بمبلغ {amount} 🪙")
    .replace("{who}", who)
    .replace("{amount}", amount)
    .replace("{name}", name)
}

pub fn funding_until(l: Lang, time: &str) -> String {
    tr!(l;
        "🌱 Funding — opens {time}", "🌱 資金注入中 — {time} 開盤", "🌱 资金注入中 — {time} 开盘", "🌱 資金提供中 — {time} に開始", "🌱 펀딩 중 — {time} 시작",
        "🌱 Фондирование — открытие {time}", "🌱 Financement — ouverture {time}", "🌱 Financiación — abre {time}", "🌱 Funding — öffnet {time}", "🌱 Đang góp vốn — mở {time}",
        "🌱 Pendanaan — buka {time}", "🌱 Pinopondohan — bubukas {time}", "🌱 กำลังระดมทุน — เปิด {time}", "🌱 Funding — opent {time}", "🌱 Fonlama — açılış {time}",
        "🌱 Financiamento — abre {time}", "🌱 फंडिंग — {time} खुलेगा", "🌱 التمويل — يفتح {time}")
    .replace("{time}", time)
}

pub fn funding_open_now(l: Lang) -> &'static str {
    tr!(l;
        "🌱 Funding closed — opens on the next trade", "🌱 資金注入結束 — 下一筆交易即開盤", "🌱 资金注入结束 — 下一笔交易即开盘", "🌱 資金提供終了 — 次の取引で開始", "🌱 펀딩 종료 — 다음 거래에서 시작",
        "🌱 Фондирование закрыто — откроется при следующей сделке", "🌱 Financement clos — ouvre à la prochaine transaction", "🌱 Financiación cerrada — abre en la próxima operación", "🌱 Funding beendet — öffnet beim nächsten Trade", "🌱 Đã đóng góp vốn — mở ở giao dịch kế tiếp",
        "🌱 Pendanaan ditutup — buka di transaksi berikutnya", "🌱 Sarado na ang funding — bubukas sa susunod na trade", "🌱 ปิดระดมทุน — เปิดเมื่อมีการเทรดถัดไป", "🌱 Funding gesloten — opent bij de volgende trade", "🌱 Fonlama kapandı — sıradaki işlemde açılır",
        "🌱 Financiamento encerrado — abre na próxima negociação", "🌱 फंडिंग बंद — अगले ट्रेड पर खुलेगा", "🌱 انتهى التمويل — يفتح عند الصفقة التالية")
}

pub fn funding_pool(l: Lang, pool: &str) -> String {
    tr!(l;
        "💧 Pool: {pool} 🪙", "💧 資金池：{pool} 🪙", "💧 资金池：{pool} 🪙", "💧 プール：{pool} 🪙", "💧 풀: {pool} 🪙",
        "💧 Пул: {pool} 🪙", "💧 Pool : {pool} 🪙", "💧 Pool: {pool} 🪙", "💧 Pool: {pool} 🪙", "💧 Pool: {pool} 🪙",
        "💧 Pool: {pool} 🪙", "💧 Pool: {pool} 🪙", "💧 พูล: {pool} 🪙", "💧 Pool: {pool} 🪙", "💧 Havuz: {pool} 🪙",
        "💧 Pool: {pool} 🪙", "💧 पूल: {pool} 🪙", "💧 المجمع: {pool} 🪙")
    .replace("{pool}", pool)
}

pub fn funding_ready(l: Lang) -> &'static str {
    tr!(l;
        "✅ Ready to open", "✅ 可開盤", "✅ 可开盘", "✅ 開始可能", "✅ 시작 가능",
        "✅ Готово к открытию", "✅ Prêt à ouvrir", "✅ Listo para abrir", "✅ Bereit zum Öffnen", "✅ Sẵn sàng mở",
        "✅ Siap dibuka", "✅ Handa nang buksan", "✅ พร้อมเปิด", "✅ Klaar om te openen", "✅ Açılmaya hazır",
        "✅ Pronto para abrir", "✅ खुलने को तैयार", "✅ جاهز للفتح")
}

pub fn funding_need(l: Lang, min: &str) -> String {
    tr!(l;
        "⚠️ Needs ≥ {min} 🪙 to open", "⚠️ 需 ≥ {min} 🪙 才能開盤", "⚠️ 需 ≥ {min} 🪙 才能开盘", "⚠️ 開始には {min} 🪙 以上が必要", "⚠️ 시작하려면 {min} 🪙 이상 필요",
        "⚠️ Нужно ≥ {min} 🪙 для открытия", "⚠️ Besoin de ≥ {min} 🪙 pour ouvrir", "⚠️ Necesita ≥ {min} 🪙 para abrir", "⚠️ Braucht ≥ {min} 🪙 zum Öffnen", "⚠️ Cần ≥ {min} 🪙 để mở",
        "⚠️ Butuh ≥ {min} 🪙 untuk buka", "⚠️ Kailangan ng ≥ {min} 🪙 para buksan", "⚠️ ต้องมี ≥ {min} 🪙 จึงเปิดได้", "⚠️ Heeft ≥ {min} 🪙 nodig om te openen", "⚠️ Açılmak için ≥ {min} 🪙 gerekir",
        "⚠️ Precisa de ≥ {min} 🪙 para abrir", "⚠️ खुलने के लिए ≥ {min} 🪙 चाहिए", "⚠️ يحتاج ≥ {min} 🪙 للفتح")
    .replace("{min}", min)
}

pub fn fund_closed(l: Lang) -> &'static str {
    tr!(l;
        "Funding has closed.", "資金注入已結束。", "资金注入已结束。", "資金提供は終了しました。", "펀딩이 마감되었습니다.",
        "Фондирование закрыто.", "Le financement est clos.", "La financiación ha cerrado.", "Das Funding ist beendet.", "Đã đóng góp vốn.",
        "Pendanaan sudah ditutup.", "Sarado na ang funding.", "ปิดระดมทุนแล้ว", "Funding is gesloten.", "Fonlama kapandı.",
        "O financiamento foi encerrado.", "फंडिंग बंद हो गई है।", "انتهى التمويل.")
}

pub fn pm_resolved(l: Lang, winner: &str) -> String {
    tr!(l;
        "✅ Resolved: {winner}\nWinnings paid out 🪙", "✅ 已結算：{winner}\n獎金已自動派發 🪙", "✅ 已结算：{winner}\n奖金已自动派发 🪙", "✅ 確定：{winner}\n配当を自動で支払ったよ 🪙", "✅ 확정: {winner}\n상금이 자동 지급됐어 🪙",
        "✅ Итог: {winner}\nВыигрыш выплачен 🪙", "✅ Résolu : {winner}\nGains versés automatiquement 🪙", "✅ Resuelto: {winner}\nGanancias pagadas 🪙", "✅ Aufgelöst: {winner}\nGewinne ausgezahlt 🪙", "✅ Đã chốt: {winner}\nĐã trả tiền thắng 🪙",
        "✅ Selesai: {winner}\nKemenangan dibayarkan 🪙", "✅ Nalutas: {winner}\nBinayad na ang panalo 🪙", "✅ ตัดสินแล้ว: {winner}\nจ่ายเงินรางวัลแล้ว 🪙", "✅ Beslist: {winner}\nWinst uitbetaald 🪙", "✅ Sonuçlandı: {winner}\nKazançlar ödendi 🪙",
        "✅ Resolvido: {winner}\nGanhos pagos 🪙", "✅ परिणाम: {winner}\nजीत का भुगतान हो गया 🪙", "✅ تم الحسم: {winner}\nتم دفع الأرباح 🪙")
    .replace("{winner}", winner)
}

pub fn pm_voided(l: Lang) -> &'static str {
    tr!(l;
        "↩️ Voided — everyone refunded 🪙", "↩️ 已作廢 — 全額退回 🪙", "↩️ 已作废 — 全额退回 🪙", "↩️ 無効 — 全額返金したよ 🪙", "↩️ 무효 — 전액 환불했어 🪙",
        "↩️ Отменено — всем возвращено 🪙", "↩️ Annulé — tout le monde remboursé 🪙", "↩️ Anulado — todos reembolsados 🪙", "↩️ Annulliert — alle erstattet 🪙", "↩️ Đã hủy — đã hoàn tiền cho mọi người 🪙",
        "↩️ Dibatalkan — semua dikembalikan 🪙", "↩️ Pinawalang-bisa — lahat na-refund 🪙", "↩️ ยกเลิก — คืนเงินทุกคนแล้ว 🪙", "↩️ Geannuleerd — iedereen terugbetaald 🪙", "↩️ İptal — herkese iade edildi 🪙",
        "↩️ Anulado — todos reembolsados 🪙", "↩️ रद्द — सभी को रिफंड 🪙", "↩️ أُلغيت — تم رد المبالغ للجميع 🪙")
}

pub fn predict_need_coins(l: Lang, coins: &str) -> String {
    tr!(l;
        "⚠️ You need {coins} 🪙 to seed this prediction's liquidity.", "⚠️ 你需要 {coins} 🪙 來提供這個預測的流動性。", "⚠️ 你需要 {coins} 🪙 来提供这个预测的流动性。", "⚠️ この予測の流動性提供に {coins} 🪙 が必要です。", "⚠️ 이 예측의 유동성 공급에 {coins} 🪙 가 필요합니다.",
        "⚠️ Нужно {coins} 🪙, чтобы обеспечить ликвидность этого прогноза.", "⚠️ Il vous faut {coins} 🪙 pour amorcer la liquidité de cette prédiction.", "⚠️ Necesitas {coins} 🪙 para aportar liquidez a esta predicción.", "⚠️ Du brauchst {coins} 🪙, um die Liquidität dieser Prognose zu stellen.", "⚠️ Bạn cần {coins} 🪙 để cấp thanh khoản cho dự đoán này.",
        "⚠️ Anda butuh {coins} 🪙 untuk menyediakan likuiditas prediksi ini.", "⚠️ Kailangan mo ng {coins} 🪙 para pondohan ang prediksyon na ito.", "⚠️ คุณต้องมี {coins} 🪙 เพื่อเติมสภาพคล่องให้การทำนายนี้", "⚠️ Je hebt {coins} 🪙 nodig om deze voorspelling van liquiditeit te voorzien.", "⚠️ Bu tahmine likidite sağlamak için {coins} 🪙 gerekir.",
        "⚠️ Você precisa de {coins} 🪙 para dar liquidez a esta previsão.", "⚠️ इस भविष्यवाणी की लिक्विडिटी के लिए आपको {coins} 🪙 चाहिए।", "⚠️ تحتاج {coins} 🪙 لتوفير السيولة لهذا التوقع.")
    .replace("{coins}", coins)
}

pub fn settle_nothing(l: Lang) -> &'static str {
    tr!(l;
        "Nothing to settle right now.", "目前沒有可結算的。", "目前没有可结算的。", "今は精算できるものがありません。", "지금 정산할 것이 없습니다.",
        "Сейчас нечего рассчитывать.", "Rien à régler pour l'instant.", "Nada que liquidar ahora.", "Im Moment nichts abzurechnen.", "Hiện chưa có gì để quyết toán.",
        "Belum ada yang bisa diselesaikan.", "Wala pang maa-settle.", "ยังไม่มีอะไรให้เคลียร์", "Nu niets af te rekenen.", "Şu an çözülecek bir şey yok.",
        "Nada para liquidar agora.", "अभी निपटाने को कुछ नहीं है।", "لا شيء للتسوية الآن.")
}

pub fn settle_done(l: Lang, events: &str, coins: &str) -> String {
    tr!(l;
        "✅ Settled {events} event(s) — paid out {coins} 🪙.", "✅ 已結算 {events} 場 — 派彩 {coins} 🪙。", "✅ 已结算 {events} 场 — 派彩 {coins} 🪙。", "✅ {events} 件を精算 — {coins} 🪙 を払い出し。", "✅ {events}건 정산 — {coins} 🪙 지급.",
        "✅ Рассчитано {events} событий — выплачено {coins} 🪙.", "✅ {events} événement(s) réglé(s) — {coins} 🪙 versés.", "✅ {events} evento(s) liquidado(s) — {coins} 🪙 pagados.", "✅ {events} Event(s) abgerechnet — {coins} 🪙 ausgezahlt.", "✅ Đã quyết toán {events} sự kiện — trả {coins} 🪙.",
        "✅ {events} event diselesaikan — dibayar {coins} 🪙.", "✅ Na-settle ang {events} event — {coins} 🪙 binayad.", "✅ เคลียร์ {events} รายการ — จ่าย {coins} 🪙", "✅ {events} event(s) afgerekend — {coins} 🪙 uitbetaald.", "✅ {events} etkinlik çözüldü — {coins} 🪙 ödendi.",
        "✅ {events} evento(s) liquidado(s) — {coins} 🪙 pagos.", "✅ {events} इवेंट निपटाए — {coins} 🪙 दिए गए।", "✅ تمت تسوية {events} حدثًا — دُفع {coins} 🪙.")
    .replace("{events}", events)
    .replace("{coins}", coins)
}

/// Self-host (`/predict`) DM stake builder — no odds (pari-mutuel).
pub fn prediction_build(l: Lang, option: &str, stake: &str) -> String {
    tr!(l;
        "🎲 Bet on {option}\nStake: {stake} 🪙\nTap to add:", "🎲 押 {option}\n下注：{stake} 🪙\n點擊加注：", "🎲 押 {option}\n下注：{stake} 🪙\n点击加注：", "🎲 {option} に賭ける\nステーク：{stake} 🪙\nタップで追加：", "🎲 {option} 에 베팅\n베팅: {stake} 🪙\n탭하여 추가:",
        "🎲 Ставка на {option}\nСтавка: {stake} 🪙\nНажмите, чтобы добавить:", "🎲 Parier sur {option}\nMise : {stake} 🪙\nAppuyez pour ajouter :", "🎲 Apostar a {option}\nApuesta: {stake} 🪙\nToca para añadir:", "🎲 Auf {option} wetten\nEinsatz: {stake} 🪙\nZum Hinzufügen tippen:", "🎲 Cược cho {option}\nĐặt: {stake} 🪙\nChạm để thêm:",
        "🎲 Bertaruh pada {option}\nTaruhan: {stake} 🪙\nKetuk untuk menambah:", "🎲 Tumaya sa {option}\nTaya: {stake} 🪙\nI-tap para magdagdag:", "🎲 เดิมพัน {option}\nเดิมพัน: {stake} 🪙\nแตะเพื่อเพิ่ม:", "🎲 Inzetten op {option}\nInzet: {stake} 🪙\nTik om toe te voegen:", "🎲 {option} için bahis\nBahis: {stake} 🪙\nEklemek için dokun:",
        "🎲 Apostar em {option}\nAposta: {stake} 🪙\nToque para adicionar:", "🎲 {option} पर दांव\nदांव: {stake} 🪙\nजोड़ने के लिए टैप करें:", "🎲 الرهان على {option}\nالرهان: {stake} 🪙\nانقر للإضافة:")
    .replace("{option}", option)
    .replace("{stake}", stake)
}

pub fn prediction_confirm(l: Lang, stake: &str, option: &str) -> String {
    tr!(l;
        "Place {stake} 🪙 on {option}?", "確定下注 {stake} 🪙 押 {option}？", "确定下注 {stake} 🪙 押 {option}？", "{option} に {stake} 🪙 を賭けますか？", "{option} 에 {stake} 🪙 베팅할까요?",
        "Поставить {stake} 🪙 на {option}?", "Miser {stake} 🪙 sur {option} ?", "¿Apostar {stake} 🪙 a {option}?", "{stake} 🪙 auf {option} setzen?", "Đặt {stake} 🪙 cho {option}?",
        "Pasang {stake} 🪙 pada {option}?", "Itaya ang {stake} 🪙 sa {option}?", "วางเดิมพัน {stake} 🪙 ที่ {option}?", "{stake} 🪙 op {option} inzetten?", "{option} için {stake} 🪙 yatırılsın mı?",
        "Apostar {stake} 🪙 em {option}?", "{option} पर {stake} 🪙 लगाएं?", "وضع {stake} 🪙 على {option}؟")
    .replace("{stake}", stake)
    .replace("{option}", option)
}

pub fn prediction_announce(l: Lang, name: &str, stake: &str, option: &str) -> String {
    tr!(l;
        "🎲 {name} bet {stake} 🪙 on {option}", "🎲 {name} 下注 {stake} 🪙 押 {option}", "🎲 {name} 下注 {stake} 🪙 押 {option}", "🎲 {name} が {option} に {stake} 🪙 を賭けた", "🎲 {name} 님이 {option} 에 {stake} 🪙 베팅",
        "🎲 {name} поставил {stake} 🪙 на {option}", "🎲 {name} a misé {stake} 🪙 sur {option}", "🎲 {name} apostó {stake} 🪙 a {option}", "🎲 {name} hat {stake} 🪙 auf {option} gesetzt", "🎲 {name} đã đặt {stake} 🪙 cho {option}",
        "🎲 {name} bertaruh {stake} 🪙 pada {option}", "🎲 {name} tumaya ng {stake} 🪙 sa {option}", "🎲 {name} เดิมพัน {stake} 🪙 ที่ {option}", "🎲 {name} zette {stake} 🪙 op {option}", "🎲 {name}, {option} için {stake} 🪙 yatırdı",
        "🎲 {name} apostou {stake} 🪙 em {option}", "🎲 {name} ने {option} पर {stake} 🪙 लगाए", "🎲 {name} راهن {stake} 🪙 على {option}")
    .replace("{name}", name)
    .replace("{stake}", stake)
    .replace("{option}", option)
}

pub fn bet_won(l: Lang, m: &str, payout: &str) -> String {
    tr!(l;
        "🎉 {market}\nYour bet won! +{payout} coins", "🎉 {market}\n你的下注贏了！+{payout} 金幣", "🎉 {market}\n你的下注赢了！+{payout} 金币", "🎉 {market}\n賭けに勝ったよ！+{payout} コイン", "🎉 {market}\n베팅에서 이겼어! +{payout} 코인",
        "🎉 {market}\nВаша ставка выиграла! +{payout} монет", "🎉 {market}\nTon pari est gagné ! +{payout} pièces", "🎉 {market}\n¡Tu apuesta ganó! +{payout} monedas", "🎉 {market}\nDeine Wette hat gewonnen! +{payout} Münzen", "🎉 {market}\nCược của bạn đã thắng! +{payout} xu",
        "🎉 {market}\nTaruhanmu menang! +{payout} koin", "🎉 {market}\nNanalo ang taya mo! +{payout} coins", "🎉 {market}\nเดิมพันของคุณชนะ! +{payout} เหรียญ", "🎉 {market}\nJe weddenschap is gewonnen! +{payout} munten", "🎉 {market}\nBahsin kazandı! +{payout} para",
        "🎉 {market}\nSua aposta ganhou! +{payout} moedas", "🎉 {market}\nआपका दांव जीत गया! +{payout} कॉइन", "🎉 {market}\nفاز رهانك! +{payout} عملة")
    .replace("{market}", m)
    .replace("{payout}", payout)
}

pub fn bet_refunded(l: Lang, m: &str, amount: &str) -> String {
    tr!(l;
        "↩️ {market}\nMarket voided — {amount} 🪙 refunded", "↩️ {market}\n盤口作廢 — 退回 {amount} 🪙", "↩️ {market}\n盘口作废 — 退回 {amount} 🪙", "↩️ {market}\nマーケットが無効に — {amount} 🪙 返金したよ", "↩️ {market}\n마켓 무효 — {amount} 🪙 환불했어",
        "↩️ {market}\nСобытие отменено — возвращено {amount} 🪙", "↩️ {market}\nMarché annulé — {amount} 🪙 remboursés", "↩️ {market}\nMercado anulado — {amount} 🪙 reembolsados", "↩️ {market}\nMarkt annulliert — {amount} 🪙 erstattet", "↩️ {market}\nThị trường bị hủy — đã hoàn {amount} 🪙",
        "↩️ {market}\nPasar dibatalkan — {amount} 🪙 dikembalikan", "↩️ {market}\nPinawalang-bisa ang market — {amount} 🪙 na-refund", "↩️ {market}\nตลาดถูกยกเลิก — คืนเงิน {amount} 🪙", "↩️ {market}\nMarkt geannuleerd — {amount} 🪙 terugbetaald", "↩️ {market}\nPiyasa iptal edildi — {amount} 🪙 iade edildi",
        "↩️ {market}\nMercado anulado — {amount} 🪙 reembolsados", "↩️ {market}\nमार्केट रद्द — {amount} 🪙 रिफंड", "↩️ {market}\nأُلغي السوق — تم رد {amount} 🪙")
    .replace("{market}", m)
    .replace("{amount}", amount)
}

// --- /history — user activity statement --------------------------------------

pub fn history_title(l: Lang) -> &'static str {
    tr!(l;
        "🧾 Your recent activity", "🧾 你的近期動態", "🧾 你的近期动态", "🧾 最近のアクティビティ", "🧾 최근 활동",
        "🧾 Ваша недавняя активность", "🧾 Ton activité récente", "🧾 Tu actividad reciente", "🧾 Deine letzten Aktivitäten", "🧾 Hoạt động gần đây của bạn",
        "🧾 Aktivitas terbaru kamu", "🧾 Iyong kamakailang aktibidad", "🧾 กิจกรรมล่าสุดของคุณ", "🧾 Je recente activiteit", "🧾 Son etkinliğin",
        "🧾 Sua atividade recente", "🧾 आपकी हाल की गतिविधि", "🧾 نشاطك الأخير")
}

pub fn history_empty(l: Lang) -> &'static str {
    tr!(l;
        "No activity yet 🫥", "還沒有任何動態 🫥", "还没有任何动态 🫥", "まだアクティビティはないよ 🫥", "아직 활동이 없어 🫥",
        "Пока нет активности 🫥", "Aucune activité pour l'instant 🫥", "Aún no hay actividad 🫥", "Noch keine Aktivität 🫥", "Chưa có hoạt động nào 🫥",
        "Belum ada aktivitas 🫥", "Wala pang aktibidad 🫥", "ยังไม่มีกิจกรรม 🫥", "Nog geen activiteit 🫥", "Henüz etkinlik yok 🫥",
        "Ainda sem atividade 🫥", "अभी कोई गतिविधि नहीं 🫥", "لا يوجد نشاط بعد 🫥")
}

/// `/history` filter-tab labels (button text + the active-tab header).
pub fn hist_tab_mining(l: Lang) -> &'static str {
    tr!(l;
        "⛏ Mining", "⛏ 挖礦", "⛏ 挖矿", "⛏ マイニング", "⛏ 채굴",
        "⛏ Майнинг", "⛏ Minage", "⛏ Minería", "⛏ Mining", "⛏ Đào coin",
        "⛏ Menambang", "⛏ Mining", "⛏ ขุดเหรียญ", "⛏ Minen", "⛏ Madencilik",
        "⛏ Mineração", "⛏ माइनिंग", "⛏ التعدين")
}

pub fn hist_tab_trading(l: Lang) -> &'static str {
    tr!(l;
        "📈 Trading", "📈 交易", "📈 交易", "📈 取引", "📈 거래",
        "📈 Торговля", "📈 Trading", "📈 Trading", "📈 Trading", "📈 Giao dịch",
        "📈 Trading", "📈 Trading", "📈 เทรด", "📈 Handel", "📈 İşlem",
        "📈 Negociação", "📈 ट्रेडिंग", "📈 التداول")
}

pub fn hist_tab_transfer(l: Lang) -> &'static str {
    tr!(l;
        "↔️ Transfer", "↔️ 轉帳", "↔️ 转账", "↔️ 送金", "↔️ 송금",
        "↔️ Перевод", "↔️ Transfert", "↔️ Transferencia", "↔️ Überweisung", "↔️ Chuyển coin",
        "↔️ Transfer", "↔️ Transfer", "↔️ โอน", "↔️ Overboeking", "↔️ Transfer",
        "↔️ Transferência", "↔️ ट्रांसफर", "↔️ تحويل")
}

pub fn hist_buy(l: Lang) -> &'static str {
    tr!(l;
        "Bought shares", "買入份額", "买入份额", "シェアを購入", "지분 매수",
        "Куплены доли", "Parts achetées", "Participaciones compradas", "Anteile gekauft", "Đã mua cổ phần",
        "Beli saham", "Bumili ng shares", "ซื้อหุ้น", "Aandelen gekocht", "Pay alındı",
        "Cotas compradas", "शेयर खरीदे", "شراء أسهم")
}

pub fn hist_sell(l: Lang) -> &'static str {
    tr!(l;
        "Sold shares", "賣出份額", "卖出份额", "シェアを売却", "지분 매도",
        "Проданы доли", "Parts vendues", "Participaciones vendidas", "Anteile verkauft", "Đã bán cổ phần",
        "Jual saham", "Nagbenta ng shares", "ขายหุ้น", "Aandelen verkocht", "Pay satıldı",
        "Cotas vendidas", "शेयर बेचे", "بيع أسهم")
}

pub fn hist_send_out(l: Lang) -> &'static str {
    tr!(l;
        "Sent coins", "轉出金幣", "转出金币", "コインを送金", "코인 보냄",
        "Отправлены монеты", "Pièces envoyées", "Monedas enviadas", "Münzen gesendet", "Đã gửi xu",
        "Koin dikirim", "Nagpadala ng coins", "ส่งเหรียญ", "Munten verzonden", "Para gönderildi",
        "Moedas enviadas", "कॉइन भेजे", "إرسال عملات")
}

pub fn hist_send_in(l: Lang) -> &'static str {
    tr!(l;
        "Received coins", "收到金幣", "收到金币", "コインを受取", "코인 받음",
        "Получены монеты", "Pièces reçues", "Monedas recibidas", "Münzen erhalten", "Đã nhận xu",
        "Koin diterima", "Nakatanggap ng coins", "รับเหรียญ", "Munten ontvangen", "Para alındı",
        "Moedas recebidas", "कॉइन मिले", "استلام عملات")
}

pub fn hist_checkin(l: Lang) -> &'static str {
    tr!(l;
        "Daily check-in", "每日簽到", "每日签到", "デイリーチェックイン", "일일 출석",
        "Ежедневный чек-ин", "Check-in quotidien", "Check-in diario", "Täglicher Check-in", "Điểm danh hằng ngày",
        "Check-in harian", "Daily check-in", "เช็คอินรายวัน", "Dagelijkse check-in", "Günlük giriş",
        "Check-in diário", "दैनिक चेक-इन", "تسجيل يومي")
}

pub fn hist_referral(l: Lang) -> &'static str {
    tr!(l;
        "Referral bonus", "推薦獎勵", "推荐奖励", "紹介ボーナス", "추천 보너스",
        "Реферальный бонус", "Bonus de parrainage", "Bono de referido", "Empfehlungsbonus", "Thưởng giới thiệu",
        "Bonus referral", "Referral bonus", "โบนัสแนะนำ", "Verwijzingsbonus", "Davet bonusu",
        "Bônus de indicação", "रेफ़रल बोनस", "مكافأة الإحالة")
}

pub fn hist_claim(l: Lang) -> &'static str {
    tr!(l;
        "Winnings", "彩金", "彩金", "配当金", "당첨금",
        "Выигрыш", "Gains", "Ganancias", "Gewinn", "Tiền thắng",
        "Kemenangan", "Panalo", "เงินรางวัล", "Winst", "Kazanç",
        "Prêmios", "जीत", "الأرباح")
}

pub fn hist_refund(l: Lang) -> &'static str {
    tr!(l;
        "Refund", "退款", "退款", "返金", "환불",
        "Возврат", "Remboursement", "Reembolso", "Rückerstattung", "Hoàn tiền",
        "Pengembalian", "Refund", "คืนเงิน", "Terugbetaling", "İade",
        "Reembolso", "रिफ़ंड", "استرداد")
}

pub fn hist_mint(l: Lang) -> &'static str {
    tr!(l;
        "Minted", "發放", "发放", "付与", "지급",
        "Начислено", "Crédité", "Acreditado", "Gutgeschrieben", "Được cấp",
        "Diberikan", "Ipinagkaloob", "เติมเหรียญ", "Toegekend", "Eklendi",
        "Creditado", "जमा किया", "منح")
}

pub fn hist_lp_fund(l: Lang) -> &'static str {
    tr!(l;
        "Provided liquidity", "提供流動性", "提供流动性", "流動性を提供", "유동성 공급",
        "Предоставлена ликвидность", "Liquidité fournie", "Liquidez aportada", "Liquidität bereitgestellt", "Đã cấp thanh khoản",
        "Menyediakan likuiditas", "Nagbigay ng liquidity", "ให้สภาพคล่อง", "Liquiditeit verstrekt", "Likidite sağlandı",
        "Liquidez fornecida", "लिक्विडिटी दी", "توفير السيولة")
}

pub fn hist_lp_return(l: Lang) -> &'static str {
    tr!(l;
        "Liquidity returned", "流動性返還", "流动性返还", "流動性の返還", "유동성 반환",
        "Ликвидность возвращена", "Liquidité restituée", "Liquidez devuelta", "Liquidität zurückgegeben", "Thanh khoản đã trả lại",
        "Likuiditas dikembalikan", "Naibalik na liquidity", "คืนสภาพคล่อง", "Liquiditeit terugbetaald", "Likidite iade edildi",
        "Liquidez devolvida", "लिक्विडिटी लौटाई", "إعادة السيولة")
}

pub fn menu_history(l: Lang) -> &'static str {
    tr!(l;
        "show your recent activity", "查看你的近期動態", "查看你的近期动态", "最近のアクティビティを見る", "최근 활동 보기",
        "показать недавнюю активность", "voir ton activité récente", "ver tu actividad reciente", "deine letzten Aktivitäten zeigen", "xem hoạt động gần đây",
        "lihat aktivitas terbaru", "ipakita ang kamakailang aktibidad", "ดูกิจกรรมล่าสุด", "je recente activiteit tonen", "son etkinliğini göster",
        "ver sua atividade recente", "अपनी हाल की गतिविधि देखें", "عرض نشاطك الأخير")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_tags() {
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("en-US"), Lang::En);
        assert_eq!(Lang::from_code("JA"), Lang::Ja);
        assert_eq!(Lang::from_code("ru"), Lang::Ru);
        assert_eq!(Lang::from_code("pt-BR"), Lang::Pt);
        assert_eq!(Lang::from_code("hi"), Lang::Hi);
        assert_eq!(Lang::from_code("ar"), Lang::Ar);
        assert_eq!(Lang::from_code("sw"), Lang::En); // unsupported → fallback
        assert_eq!(Lang::from_code(""), Lang::En);
        assert_eq!(Lang::from_code("fil"), Lang::Fil);
        assert_eq!(Lang::from_code("tl"), Lang::Fil);
    }

    #[test]
    fn splits_chinese_by_script_and_region() {
        assert_eq!(Lang::from_code("zh-Hant"), Lang::Hant);
        assert_eq!(Lang::from_code("zh-TW"), Lang::Hant);
        assert_eq!(Lang::from_code("zh-HK"), Lang::Hant);
        assert_eq!(Lang::from_code("zh-Hans"), Lang::Hans);
        assert_eq!(Lang::from_code("zh-CN"), Lang::Hans);
        assert_eq!(Lang::from_code("zh"), Lang::Hans); // bare zh → Simplified
    }

    #[test]
    fn no_unfilled_placeholders_in_any_locale() {
        // Every parameterised template, in every locale, must consume all of
        // its tokens — a leftover `{` means an arm forgot a placeholder.
        for l in Lang::ALL {
            let samples = [
                sent_coins(l, "A", "B", "1"),
                sent_envelope_title(l, "A", "1"),
                sent_fruits(l, "A", "B", "🍎"),
                thanks(l, "A", "x"),
                sell_button(l, "1"),
                buy_button(l, "1"),
                sell_listing(l, "A", "🍎", "1"),
                buy_listing(l, "A", "🍎", "1"),
                received_fruit(l, "A", "🍎"),
                received_coins(l, "A", "1"),
                bet_pending(l, "5", "X"),
                bought_msg(l, "A", "1", "🍎"),
                bought_toast(l, "🍎"),
                sold_msg(l, "A", "🍎", "1"),
                sold_toast(l, "1"),
                sell_pnl(l, "📈", "+5"),
                you_dont_have(l, "🍎"),
                result_header(l, "X"),
                settle_line(l, "Alice", verb_won(l), "5"),
                board_footer_open(l, "35"),
                board_footer_closed(l, "35"),
                markets_page(l, "1", "3"),
                checkin_done(l, "10"),
                menu_status(l, "10"),
                checkin_already(l, "1h 2m"),
                invite_count(l, "3"),
                invite_copy(l, "link"),
                invite_forward(l, "link"),
                referral_bonus(l, "A", "20"),
                bet_build(l, "A", "1.54", "5", "8"),
                bet_confirm(l, "5", "A", "8"),
                bet_placed(l, "10", "A", "1.54", "15"),
                bet_won(l, "A vs. B / A", "15"),
                bet_lost(l, "A vs. B / A"),
            ];
            for s in samples {
                assert!(!s.contains('{'), "unfilled placeholder in {l:?}: {s}");
            }
        }
    }
}
