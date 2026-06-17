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
//! shown to everyone. `BetGame` therefore stores a [`Lang`] (see `game.rs`).
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
/// `BetGame` persists a `Lang` to SQLite as JSON.
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

pub fn hi(l: Lang) -> &'static str {
    tr!(l;
        "Hi?", "嗨？", "嗨？", "やあ？", "안녕?",
        "Привет?", "Salut ?", "¿Hola?", "Hallo?", "Chào?",
        "Hai?", "Kumusta?", "หวัดดี?", "Hoi?", "Selam?",
        "Olá?", "नमस्ते?", "مرحبا؟")
}

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

pub fn no_betting_records(l: Lang) -> &'static str {
    tr!(l;
        "No betting records 🤗", "沒有賭博記錄🤗", "没有赌博记录🤗", "賭けの記録はないよ🤗", "베팅 기록이 없어🤗",
        "Нет ставок 🤗", "Aucun pari en cours 🤗", "Sin registros de apuestas 🤗", "Keine Wetten 🤗", "Không có lịch sử cá cược 🤗",
        "Tidak ada catatan taruhan 🤗", "Walang record ng pusta 🤗", "ไม่มีประวัติการเดิมพัน 🤗", "Geen weddenschappen 🤗", "Bahis kaydı yok 🤗",
        "Nenhum registro de apostas 🤗", "कोई बेटिंग रिकॉर्ड नहीं 🤗", "لا سجلّات رهان 🤗")
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

pub fn game_invalid(l: Lang) -> &'static str {
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

pub fn close_game_toast(l: Lang) -> &'static str {
    tr!(l;
        "Game closed", "關閉賭局", "关闭赌局", "賭けを締め切りました", "베팅 마감",
        "Ставки закрыты", "Paris clôturés", "Apuestas cerradas", "Wetten geschlossen", "Đã đóng cược",
        "Taruhan ditutup", "Sarado na ang pusta", "ปิดรับเดิมพันแล้ว", "Inzetten gesloten", "Bahisler kapandı",
        "Apostas encerradas", "बेटिंग बंद", "أُغلق الرهان")
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
        "Draw", "流局", "流局", "流局", "무승부",
        "Ничья", "Nul", "Empate", "Unentschieden", "Hòa",
        "Seri", "Patas", "เสมอ", "Gelijkspel", "Berabere",
        "Empate", "ड्रॉ", "تعادل")
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

pub fn have_coins(l: Lang, name: &str, coins: &str) -> String {
    tr!(l;
        "{name}\nhas {coins} water-coins", "{name}\n擁有 {coins} 顆 水幣", "{name}\n拥有 {coins} 颗 水币", "{name}\n水コインを {coins} 枚 持っている", "{name}\n물코인 {coins} 개 보유",
        "{name}\nимеет {coins} водных монет", "{name}\npossède {coins} pièces d'eau", "{name}\ntiene {coins} monedas de agua", "{name}\nhat {coins} Wassermünzen", "{name}\ncó {coins} xu nước",
        "{name}\npunya {coins} koin air", "{name}\nmay {coins} water-coins", "{name}\nมีเหรียญน้ำ {coins} เหรียญ", "{name}\nheeft {coins} watermunten", "{name}\n{coins} su parası var",
        "{name}\ntem {coins} moedas de água", "{name}\nके पास {coins} वॉटर-कॉइन हैं", "{name}\nلديه {coins} عملة مائية")
    .replace("{name}", name)
    .replace("{coins}", coins)
}

pub fn debt_coins(l: Lang, name: &str, coins: &str) -> String {
    tr!(l;
        "{name}\nis in debt {coins} water-coins", "{name}\n欠債 {coins} 顆 水幣", "{name}\n欠债 {coins} 颗 水币", "{name}\n水コインを {coins} 枚 借金している", "{name}\n물코인 {coins} 개 빚짐",
        "{name}\nдолжен {coins} водных монет", "{name}\ndoit {coins} pièces d'eau", "{name}\ndebe {coins} monedas de agua", "{name}\nschuldet {coins} Wassermünzen", "{name}\nđang nợ {coins} xu nước",
        "{name}\nberutang {coins} koin air", "{name}\nmay utang na {coins} water-coins", "{name}\nเป็นหนี้เหรียญน้ำ {coins} เหรียญ", "{name}\nstaat {coins} watermunten in het rood", "{name}\n{coins} su parası borçlu",
        "{name}\nestá devendo {coins} moedas de água", "{name}\nपर {coins} वॉटर-कॉइन का कर्ज़ है", "{name}\nمَدين بـ {coins} عملة مائية")
    .replace("{name}", name)
    .replace("{coins}", coins)
}

pub fn want_fruit(l: Lang, name: &str) -> String {
    tr!(l;
        "{name}\nwants some fruit 🤤", "{name}\n想要水果🤤", "{name}\n想要水果🤤", "{name}\nフルーツが欲しい🤤", "{name}\n과일이 먹고 싶어🤤",
        "{name}\nхочет фруктов 🤤", "{name}\nveut des fruits 🤤", "{name}\nquiere fruta 🤤", "{name}\nmöchte Obst 🤤", "{name}\nthèm trái cây 🤤",
        "{name}\nmau buah 🤤", "{name}\ngustong-gusto ng prutas 🤤", "{name}\nอยากกินผลไม้ 🤤", "{name}\nwil wat fruit 🤤", "{name}\nbiraz meyve istiyor 🤤",
        "{name}\nquer um pouco de fruta 🤤", "{name}\nको कुछ फल चाहिए 🤤", "{name}\nيريد بعض الفاكهة 🤤")
    .replace("{name}", name)
}

pub fn fruit_store(l: Lang, name: &str, fruits: &str) -> String {
    tr!(l;
        "{name}'s fruit stash:\n{fruits}", "{name}\n的水果庫:\n{fruits}", "{name}\n的水果库:\n{fruits}", "{name} のフルーツ庫:\n{fruits}", "{name} 의 과일 창고:\n{fruits}",
        "Запас фруктов {name}:\n{fruits}", "Réserve de fruits de {name} :\n{fruits}", "Reserva de fruta de {name}:\n{fruits}", "Obstvorrat von {name}:\n{fruits}", "Kho trái cây của {name}:\n{fruits}",
        "Stok buah {name}:\n{fruits}", "Imbak na prutas ni {name}:\n{fruits}", "คลังผลไม้ของ {name}:\n{fruits}", "Fruitvoorraad van {name}:\n{fruits}", "{name} kullanıcısının meyve deposu:\n{fruits}",
        "Estoque de frutas de {name}:\n{fruits}", "{name} का फल भंडार:\n{fruits}", "مخزون فاكهة {name}:\n{fruits}")
    .replace("{name}", name)
    .replace("{fruits}", fruits)
}

pub fn sent_coins(l: Lang, sender: &str, recv: &str, coins: &str) -> String {
    tr!(l;
        "{sender} sent {recv}\n{coins} water-coins", "{sender} 送給 {recv}\n{coins} 顆 水幣", "{sender} 送给 {recv}\n{coins} 颗 水币", "{sender} が {recv} に\n水コインを {coins} 枚 送った", "{sender} 님이 {recv} 님에게\n물코인 {coins} 개 보냄",
        "{sender} отправил {recv}\n{coins} водных монет", "{sender} a envoyé à {recv}\n{coins} pièces d'eau", "{sender} envió a {recv}\n{coins} monedas de agua", "{sender} hat {recv}\n{coins} Wassermünzen geschickt", "{sender} đã gửi {recv}\n{coins} xu nước",
        "{sender} mengirim {recv}\n{coins} koin air", "Nagpadala si {sender} kay {recv}\nng {coins} water-coins", "{sender} ส่งให้ {recv}\nเหรียญน้ำ {coins} เหรียญ", "{sender} stuurde {recv}\n{coins} watermunten", "{sender}, {recv} kullanıcısına\n{coins} su parası gönderdi",
        "{sender} enviou a {recv}\n{coins} moedas de água", "{sender} ने {recv} को\n{coins} वॉटर-कॉइन भेजे", "{sender} أرسل إلى {recv}\n{coins} عملة مائية")
    .replace("{sender}", sender)
    .replace("{recv}", recv)
    .replace("{coins}", coins)
}

pub fn sent_envelope_title(l: Lang, sender: &str, coins: &str) -> String {
    tr!(l;
        "{sender} dropped a {coins} water-coin red envelope!", "{sender} 發紅包 {coins} 水幣！", "{sender} 发红包 {coins} 水币！", "{sender} が {coins} 水コインの紅包を配った！", "{sender} 님이 {coins} 물코인 행운 봉투를 뿌렸어요!",
        "{sender} раздаёт красный конверт на {coins} водных монет!", "{sender} lâche une enveloppe rouge de {coins} pièces d'eau !", "¡{sender} soltó un sobre rojo de {coins} monedas de agua!", "{sender} verteilt einen roten Umschlag mit {coins} Wassermünzen!", "{sender} phát lì xì {coins} xu nước!",
        "{sender} membagikan amplop merah {coins} koin air!", "Naghulog si {sender} ng red envelope na {coins} water-coins!", "{sender} แจกซองแดง {coins} เหรียญน้ำ!", "{sender} deelt een rode envelop van {coins} watermunten uit!", "{sender}, {coins} su paralık kırmızı zarf bıraktı!",
        "{sender} soltou um envelope vermelho de {coins} moedas de água!", "{sender} ने {coins} वॉटर-कॉइन का लाल लिफ़ाफ़ा छोड़ा!", "{sender} أسقط مظروفًا أحمر بـ {coins} عملة مائية!")
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
        "{seller} is selling {fruits}\nasking {price} water-coins", "{seller} 出售 {fruits}\n要價 {price} 水幣", "{seller} 出售 {fruits}\n要价 {price} 水币", "{seller} が {fruits} を売り出し\n希望価格 {price} 水コイン", "{seller} 님이 {fruits} 판매\n희망가 {price} 물코인",
        "{seller} продаёт {fruits}\nцена {price} водных монет", "{seller} vend {fruits}\nprix demandé {price} pièces d'eau", "{seller} vende {fruits}\npide {price} monedas de agua", "{seller} verkauft {fruits}\nfür {price} Wassermünzen", "{seller} đang bán {fruits}\ngiá {price} xu nước",
        "{seller} menjual {fruits}\nharga {price} koin air", "Nagbebenta si {seller} ng {fruits}\nhinihingi {price} water-coins", "{seller} ขาย {fruits}\nราคา {price} เหรียญน้ำ", "{seller} verkoopt {fruits}\nvraagprijs {price} watermunten", "{seller}, {fruits} satıyor\nistenen fiyat {price} su parası",
        "{seller} está vendendo {fruits}\npedindo {price} moedas de água", "{seller} {fruits} बेच रहे हैं\nमाँग {price} वॉटर-कॉइन", "{seller} يبيع {fruits}\nالسعر المطلوب {price} عملة مائية")
    .replace("{seller}", seller)
    .replace("{fruits}", fruits)
    .replace("{price}", price)
}

pub fn buy_listing(l: Lang, buyer: &str, fruits: &str, price: &str) -> String {
    tr!(l;
        "{buyer} wants to buy {fruits}\noffering {price} water-coins", "{buyer} 收購 {fruits}\n出價 {price} 水幣", "{buyer} 收购 {fruits}\n出价 {price} 水币", "{buyer} が {fruits} を買い取り\n提示額 {price} 水コイン", "{buyer} 님이 {fruits} 매입\n제시가 {price} 물코인",
        "{buyer} скупает {fruits}\nпредлагает {price} водных монет", "{buyer} achète {fruits}\noffre {price} pièces d'eau", "{buyer} compra {fruits}\nofrece {price} monedas de agua", "{buyer} kauft {fruits}\nbietet {price} Wassermünzen", "{buyer} thu mua {fruits}\ntrả {price} xu nước",
        "{buyer} membeli {fruits}\nmenawar {price} koin air", "Bumibili si {buyer} ng {fruits}\nnag-aalok ng {price} water-coins", "{buyer} รับซื้อ {fruits}\nเสนอ {price} เหรียญน้ำ", "{buyer} koopt {fruits}\nbiedt {price} watermunten", "{buyer}, {fruits} alıyor\n{price} su parası teklif ediyor",
        "{buyer} quer comprar {fruits}\noferecendo {price} moedas de água", "{buyer} {fruits} खरीदना चाहते हैं\nप्रस्ताव {price} वॉटर-कॉइन", "{buyer} يريد شراء {fruits}\nيعرض {price} عملة مائية")
    .replace("{buyer}", buyer)
    .replace("{fruits}", fruits)
    .replace("{price}", price)
}

pub fn received_fruit(l: Lang, name: &str, fruit: &str) -> String {
    tr!(l;
        "{name} received a {fruit}", "{name} 收到一顆 {fruit}", "{name} 收到一颗 {fruit}", "{name} が {fruit} を1つ受け取った", "{name} 님이 {fruit} 한 개 받음",
        "{name} получил {fruit}", "{name} a reçu un {fruit}", "{name} recibió un {fruit}", "{name} hat ein {fruit} erhalten", "{name} nhận được một {fruit}",
        "{name} menerima sebuah {fruit}", "Nakatanggap si {name} ng {fruit}", "{name} ได้รับ {fruit} หนึ่งลูก", "{name} ontving een {fruit}", "{name} bir {fruit} aldı",
        "{name} recebeu um {fruit}", "{name} को एक {fruit} मिला", "{name} حصل على {fruit}")
    .replace("{name}", name)
    .replace("{fruit}", fruit)
}

pub fn received_coins(l: Lang, name: &str, coins: &str) -> String {
    tr!(l;
        "{name} received {coins} water-coins", "{name} 收到 {coins} 顆 水幣", "{name} 收到 {coins} 颗 水币", "{name} が水コインを {coins} 枚 受け取った", "{name} 님이 물코인 {coins} 개 받음",
        "{name} получил {coins} водных монет", "{name} a reçu {coins} pièces d'eau", "{name} recibió {coins} monedas de agua", "{name} hat {coins} Wassermünzen erhalten", "{name} nhận được {coins} xu nước",
        "{name} menerima {coins} koin air", "Nakatanggap si {name} ng {coins} water-coins", "{name} ได้รับ {coins} เหรียญน้ำ", "{name} ontving {coins} watermunten", "{name}, {coins} su parası aldı",
        "{name} recebeu {coins} moedas de água", "{name} को {coins} वॉटर-कॉइन मिले", "{name} حصل على {coins} عملة مائية")
    .replace("{name}", name)
    .replace("{coins}", coins)
}

pub fn bets_for_option(l: Lang, opt: &str) -> String {
    tr!(l;
        "Bets on {opt}", "{opt} 的押注", "{opt} 的押注", "{opt} への賭け", "{opt} 에 대한 베팅",
        "Ставки на {opt}", "Mises sur {opt}", "Apuestas a {opt}", "Wetten auf {opt}", "Cược cho {opt}",
        "Taruhan untuk {opt}", "Pusta para sa {opt}", "เดิมพันสำหรับ {opt}", "Inzetten op {opt}", "{opt} için bahisler",
        "Apostas em {opt}", "{opt} पर दांव", "رهانات على {opt}")
    .replace("{opt}", opt)
}

pub fn bought_msg(l: Lang, name: &str, price: &str, fruits: &str) -> String {
    tr!(l;
        "{name} spent {price} water-coins\nand bought {fruits}", "{name} 花 {price} 水幣\n買了 {fruits}", "{name} 花 {price} 水币\n买了 {fruits}", "{name} が水コインを {price} 枚 使って\n{fruits} を買った", "{name} 님이 물코인 {price} 개 써서\n{fruits} 구매",
        "{name} потратил {price} водных монет\nи купил {fruits}", "{name} a dépensé {price} pièces d'eau\net acheté {fruits}", "{name} gastó {price} monedas de agua\ny compró {fruits}", "{name} hat {price} Wassermünzen ausgegeben\nund {fruits} gekauft", "{name} đã tiêu {price} xu nước\nvà mua {fruits}",
        "{name} menghabiskan {price} koin air\ndan membeli {fruits}", "Gumastos si {name} ng {price} water-coins\nat bumili ng {fruits}", "{name} จ่าย {price} เหรียญน้ำ\nและซื้อ {fruits}", "{name} gaf {price} watermunten uit\nen kocht {fruits}", "{name}, {price} su parası harcayıp\n{fruits} aldı",
        "{name} gastou {price} moedas de água\ne comprou {fruits}", "{name} ने {price} वॉटर-कॉइन खर्च कर\n{fruits} खरीदा", "{name} أنفق {price} عملة مائية\nواشترى {fruits}")
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
        "{name} sold {fruits}\nand earned {price} water-coins", "{name} 賣出 {fruits}\n賺了 {price} 水幣", "{name} 卖出 {fruits}\n赚了 {price} 水币", "{name} が {fruits} を売って\n水コインを {price} 枚 稼いだ", "{name} 님이 {fruits} 팔아서\n물코인 {price} 개 벌었어요",
        "{name} продал {fruits}\nи заработал {price} водных монет", "{name} a vendu {fruits}\net gagné {price} pièces d'eau", "{name} vendió {fruits}\ny ganó {price} monedas de agua", "{name} hat {fruits} verkauft\nund {price} Wassermünzen verdient", "{name} đã bán {fruits}\nvà kiếm được {price} xu nước",
        "{name} menjual {fruits}\ndan mendapat {price} koin air", "Ibinenta ni {name} ang {fruits}\nat kumita ng {price} water-coins", "{name} ขาย {fruits}\nและได้ {price} เหรียญน้ำ", "{name} verkocht {fruits}\nen verdiende {price} watermunten", "{name}, {fruits} satıp\n{price} su parası kazandı",
        "{name} vendeu {fruits}\ne ganhou {price} moedas de água", "{name} ने {fruits} बेचा\nऔर {price} वॉटर-कॉइन कमाए", "{name} باع {fruits}\nوربح {price} عملة مائية")
    .replace("{name}", name)
    .replace("{fruits}", fruits)
    .replace("{price}", price)
}

pub fn sold_toast(l: Lang, price: &str) -> String {
    tr!(l;
        "Earned {price} water-coins 🥳", "賺取 {price} 水幣🥳", "赚取 {price} 水币🥳", "水コインを {price} 枚 稼いだ🥳", "물코인 {price} 개 벌었다🥳",
        "Заработано {price} водных монет 🥳", "{price} pièces d'eau gagnées 🥳", "Ganaste {price} monedas de agua 🥳", "{price} Wassermünzen verdient 🥳", "Kiếm được {price} xu nước 🥳",
        "Dapat {price} koin air 🥳", "Kumita ng {price} water-coins 🥳", "ได้ {price} เหรียญน้ำ 🥳", "{price} watermunten verdiend 🥳", "{price} su parası kazanıldı 🥳",
        "Ganhou {price} moedas de água 🥳", "{price} वॉटर-कॉइन कमाए 🥳", "ربح {price} عملة مائية 🥳")
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

pub fn result_header(l: Lang, id: &str, outcome: &str) -> String {
    tr!(l;
        "{id}\nHost's result: {outcome}", "{id}\n莊家指定結果：{outcome}", "{id}\n庄家指定结果：{outcome}", "{id}\n親の指定結果：{outcome}", "{id}\n딜러 지정 결과: {outcome}",
        "{id}\nИтог от ведущего: {outcome}", "{id}\nRésultat de l'organisateur : {outcome}", "{id}\nResultado del anfitrión: {outcome}", "{id}\nErgebnis des Gastgebers: {outcome}", "{id}\nKết quả nhà cái: {outcome}",
        "{id}\nHasil dari bandar: {outcome}", "{id}\nResulta ng host: {outcome}", "{id}\nผลที่เจ้ามือกำหนด: {outcome}", "{id}\nUitslag van spelleider: {outcome}", "{id}\nEv sahibinin sonucu: {outcome}",
        "{id}\nResultado do anfitrião: {outcome}", "{id}\nहोस्ट का परिणाम: {outcome}", "{id}\nنتيجة المُضيف: {outcome}")
    .replace("{id}", id)
    .replace("{outcome}", outcome)
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
pub fn command_menu(l: Lang) -> [(&'static str, &'static str); 9] {
    [
        ("start", hi(l)),
        ("balance", menu_balance(l)),
        ("fruit", menu_fruit(l)),
        ("send", menu_send(l)),
        ("host", menu_host(l)),
        ("sell", menu_sell(l)),
        ("buy", menu_buy(l)),
        ("markets", menu_markets(l)),
        ("checkin", menu_checkin(l)),
    ]
}

fn menu_checkin(l: Lang) -> &'static str {
    tr!(l;
        "Claim your daily 10 water-coins", "領取每日 10 水幣", "领取每日 10 水币", "毎日10水コインを受け取る", "매일 10 물코인 받기",
        "Ежедневные 10 водных монет", "Réclame tes 10 pièces d'eau du jour", "Reclama tus 10 monedas de agua diarias", "Tägliche 10 Wassermünzen abholen", "Nhận 10 xu nước mỗi ngày",
        "Klaim 10 koin air harian", "Kunin ang araw-araw na 10 water-coins", "รับ 10 เหรียญน้ำประจำวัน", "Claim je dagelijkse 10 watermunten", "Günlük 10 su paranı al",
        "Resgate 10 moedas de água diárias", "रोज़ाना 10 वॉटर-कॉइन लें", "احصل على 10 عملات مائية يوميًا")
}

fn menu_markets(l: Lang) -> &'static str {
    tr!(l;
        "Browse live prediction markets", "瀏覽即時預測市場", "浏览实时预测市场", "予測市場をチェック", "실시간 예측 마켓 보기",
        "Прогнозные рынки", "Voir les marchés de prédiction", "Ver mercados de predicción", "Prognosemärkte ansehen", "Xem thị trường dự đoán",
        "Lihat pasar prediksi", "Tingnan ang prediction markets", "ดูตลาดทำนายผล", "Bekijk voorspellingsmarkten", "Tahmin piyasalarını gör",
        "Ver mercados de previsão", "प्रिडिक्शन मार्केट देखें", "تصفّح أسواق التنبؤ")
}

fn menu_balance(l: Lang) -> &'static str {
    tr!(l;
        "Check your water-coin balance", "查看水幣餘額", "查看水币余额", "水コインの残高を見る", "물코인 잔액 확인",
        "Посмотреть баланс водных монет", "Voir ton solde de pièces d'eau", "Ver tu saldo de monedas de agua", "Wassermünzen-Guthaben ansehen", "Xem số dư xu nước",
        "Cek saldo koin air", "Tingnan ang water-coin balance", "ดูยอดเหรียญน้ำ", "Bekijk je watermunten-saldo", "Su parası bakiyeni gör",
        "Veja seu saldo de moedas de água", "अपना वॉटर-कॉइन बैलेंस देखें", "اطّلع على رصيد عملاتك المائية")
}

fn menu_fruit(l: Lang) -> &'static str {
    tr!(l;
        "Check your fruit", "查看水果", "查看水果", "フルーツを見る", "과일 확인",
        "Посмотреть фрукты", "Voir tes fruits", "Ver tu fruta", "Dein Obst ansehen", "Xem trái cây",
        "Cek buahmu", "Tingnan ang iyong prutas", "ดูผลไม้ของคุณ", "Bekijk je fruit", "Meyvelerini gör",
        "Veja suas frutas", "अपने फल देखें", "اطّلع على فاكهتك")
}

fn menu_send(l: Lang) -> &'static str {
    tr!(l;
        "Reply to a message to send coins or fruit", "回覆訊息以送出水幣或水果", "回复消息以送出水币或水果", "メッセージに返信してコインや果物を送る", "메시지에 답장해 코인이나 과일 보내기",
        "Ответьте на сообщение, чтобы отправить монеты или фрукты", "Réponds à un message pour envoyer pièces ou fruits", "Responde a un mensaje para enviar monedas o fruta", "Auf eine Nachricht antworten, um Münzen oder Obst zu senden", "Trả lời tin nhắn để gửi xu hoặc trái cây",
        "Balas pesan untuk mengirim koin atau buah", "Mag-reply para magpadala ng coins o prutas", "ตอบกลับข้อความเพื่อส่งเหรียญหรือผลไม้", "Reageer op een bericht om munten of fruit te sturen", "Coin veya meyve göndermek için bir mesaja yanıt ver",
        "Responda a uma mensagem para enviar moedas ou frutas", "सिक्के या फल भेजने के लिए संदेश का जवाब दें", "ردّ على رسالة لإرسال عملات أو فاكهة")
}

fn menu_host(l: Lang) -> &'static str {
    tr!(l;
        "Host a bet or view your stakes", "開賭局或查看自己押注", "开赌局或查看自己押注", "賭けを開くか自分の賭けを見る", "베팅을 열거나 내 베팅 보기",
        "Открыть ставку или посмотреть свои", "Ouvrir un pari ou voir tes mises", "Abre una apuesta o mira las tuyas", "Eine Wette eröffnen oder deine Einsätze ansehen", "Mở ván cược hoặc xem cược của bạn",
        "Buka taruhan atau lihat taruhanmu", "Magbukas ng pusta o tingnan ang sa'yo", "เปิดเดิมพันหรือดูเดิมพันของคุณ", "Open een weddenschap of bekijk je inzetten", "Bahis aç ya da bahislerini gör",
        "Abra uma aposta ou veja as suas", "बेट खोलें या अपने दांव देखें", "افتح رهانًا أو اطّلع على رهاناتك")
}

fn menu_sell(l: Lang) -> &'static str {
    tr!(l;
        "/sell <fruit> <price>", "/sell <水果> <價格>", "/sell <水果> <价格>", "/sell <フルーツ> <価格>", "/sell <과일> <가격>",
        "/sell <фрукт> <цена>", "/sell <fruit> <prix>", "/sell <fruta> <precio>", "/sell <Obst> <Preis>", "/sell <trái cây> <giá>",
        "/sell <buah> <harga>", "/sell <prutas> <presyo>", "/sell <ผลไม้> <ราคา>", "/sell <fruit> <prijs>", "/sell <meyve> <fiyat>",
        "/sell <fruta> <preço>", "/sell <फल> <मूल्य>", "/sell <فاكهة> <سعر>")
}

fn menu_buy(l: Lang) -> &'static str {
    tr!(l;
        "/buy <fruit> <price>", "/buy <水果> <價格>", "/buy <水果> <价格>", "/buy <フルーツ> <価格>", "/buy <과일> <가격>",
        "/buy <фрукт> <цена>", "/buy <fruit> <prix>", "/buy <fruta> <precio>", "/buy <Obst> <Preis>", "/buy <trái cây> <giá>",
        "/buy <buah> <harga>", "/buy <prutas> <presyo>", "/buy <ผลไม้> <ราคา>", "/buy <fruit> <prijs>", "/buy <meyve> <fiyat>",
        "/buy <fruta> <preço>", "/buy <फल> <मूल्य>", "/buy <فاكهة> <سعر>")
}

/// Per-staker settlement line, e.g. `***1234 won 50 water-coins`. `verb` is
/// already localized via [`verb_won`] / [`verb_lost`].
pub fn settle_line(l: Lang, tail: &str, verb: &str, amt: &str) -> String {
    tr!(l;
        "\n***{tail} {verb} {amt} water-coins", "\n***{tail} {verb}{amt}顆 水幣", "\n***{tail} {verb}{amt}颗 水币", "\n***{tail} 水コイン{amt}枚{verb}", "\n***{tail} 물코인 {amt}개 {verb}",
        "\n***{tail} {verb} {amt} водных монет", "\n***{tail} {verb} {amt} pièces d'eau", "\n***{tail} {verb} {amt} monedas de agua", "\n***{tail} {verb} {amt} Wassermünzen", "\n***{tail} {verb} {amt} xu nước",
        "\n***{tail} {verb} {amt} koin air", "\n***{tail} {verb} {amt} water-coins", "\n***{tail} {verb} {amt} เหรียญน้ำ", "\n***{tail} {verb} {amt} watermunten", "\n***{tail} {amt} su parası {verb}",
        "\n***{tail} {verb} {amt} moedas de água", "\n***{tail} {amt} वॉटर-कॉइन {verb}", "\n***{tail} {verb} {amt} عملة مائية")
    .replace("{tail}", tail)
    .replace("{verb}", verb)
    .replace("{amt}", amt)
}

// ----------------------------------------------------------------------------
// Admin commands (owner-only)
// ----------------------------------------------------------------------------

pub fn minted(l: Lang, name: &str, amt: &str) -> String {
    tr!(l;
        "🪄 Minted {amt} to {name}", "🪄 已給 {name} 鑄造 {amt}", "🪄 已给 {name} 铸造 {amt}", "🪄 {name} に {amt} を発行したよ", "🪄 {name}에게 {amt} 발행했어",
        "🪄 Начислено {amt} для {name}", "🪄 {amt} créés pour {name}", "🪄 Acuñado {amt} para {name}", "🪄 {amt} für {name} erzeugt", "🪄 Đã đúc {amt} cho {name}",
        "🪄 Mint {amt} ke {name}", "🪄 Nag-mint ng {amt} kay {name}", "🪄 มินต์ {amt} ให้ {name} แล้ว", "🪄 {amt} naar {name} gemunt", "🪄 {name} için {amt} basıldı",
        "🪄 Cunhado {amt} para {name}", "🪄 {name} को {amt} मिंट किया", "🪄 تم سكّ {amt} لـ {name}")
    .replace("{amt}", amt)
    .replace("{name}", name)
}

pub fn mint_usage(l: Lang) -> &'static str {
    tr!(l;
        "Reply to someone with /mint <amount> 🪄", "回覆對方並輸入 /mint <數量> 🪄", "回复对方并输入 /mint <数量> 🪄", "相手に返信して /mint <数量> 🪄", "상대에게 답장하고 /mint <수량> 🪄",
        "Ответьте на сообщение: /mint <сумма> 🪄", "Réponds à quelqu'un avec /mint <montant> 🪄", "Responde a alguien con /mint <cantidad> 🪄", "Antworte jemandem mit /mint <Betrag> 🪄", "Trả lời ai đó với /mint <số lượng> 🪄",
        "Balas seseorang dengan /mint <jumlah> 🪄", "Mag-reply gamit ang /mint <halaga> 🪄", "ตอบกลับใครสักคนด้วย /mint <จำนวน> 🪄", "Reageer met /mint <bedrag> 🪄", "Birine /mint <miktar> ile yanıt ver 🪄",
        "Responda a alguém com /mint <quantia> 🪄", "किसी को /mint <राशि> से जवाब दें 🪄", "ردّ على أحدهم بـ /mint <المبلغ> 🪄")
}

pub fn broadcast_sent(l: Lang, n: &str) -> String {
    tr!(l;
        "📣 Broadcast sent to {n} chats", "📣 已廣播給 {n} 個對話", "📣 已广播给 {n} 个对话", "📣 {n} 件のチャットに配信したよ", "📣 {n}개 채팅에 전송했어",
        "📣 Рассылка отправлена в {n} чатов", "📣 Diffusé à {n} discussions", "📣 Enviado a {n} chats", "📣 An {n} Chats gesendet", "📣 Đã gửi tới {n} cuộc trò chuyện",
        "📣 Disiarkan ke {n} chat", "📣 Naipadala sa {n} chat", "📣 ส่งถึง {n} แชทแล้ว", "📣 Verzonden naar {n} chats", "📣 {n} sohbete gönderildi",
        "📣 Transmitido para {n} conversas", "📣 {n} चैट में भेजा गया", "📣 أُرسل إلى {n} محادثة")
    .replace("{n}", n)
}

pub fn broadcast_usage(l: Lang) -> &'static str {
    tr!(l;
        "/broadcast <message>", "/broadcast <訊息>", "/broadcast <消息>", "/broadcast <メッセージ>", "/broadcast <메시지>",
        "/broadcast <сообщение>", "/broadcast <message>", "/broadcast <mensaje>", "/broadcast <Nachricht>", "/broadcast <tin nhắn>",
        "/broadcast <pesan>", "/broadcast <mensahe>", "/broadcast <ข้อความ>", "/broadcast <bericht>", "/broadcast <mesaj>",
        "/broadcast <mensagem>", "/broadcast <संदेश>", "/broadcast <رسالة>")
}

// ----------------------------------------------------------------------------
// `/start` onboarding & main menu
// ----------------------------------------------------------------------------

/// Neutral, language-agnostic prompt shown before the user has picked a locale.
pub const CHOOSE_LANGUAGE: &str =
    "🌐 Please choose your language\n请选择语言 · 言語を選択 · 언어 선택";

pub fn intro(l: Lang) -> &'static str {
    tr!(l;
        "Hi, I'm Xaliah. Nice to meet you 😊", "嗨，我是 Xaliah，很高興認識你 😊", "嗨，我是 Xaliah，很高兴认识你 😊", "やあ、私は Xaliah。会えて嬉しいよ 😊", "안녕, 나는 Xaliah야. 만나서 반가워 😊",
        "Привет, я Xaliah. Рада знакомству 😊", "Salut, je suis Xaliah. Ravie de te rencontrer 😊", "Hola, soy Xaliah. Encantada de conocerte 😊", "Hi, ich bin Xaliah. Schön dich kennenzulernen 😊", "Chào, mình là Xaliah. Rất vui được gặp bạn 😊",
        "Hai, aku Xaliah. Senang berkenalan 😊", "Hi, ako si Xaliah. Ikinagagalak kitang makilala 😊", "สวัสดี ฉันชื่อ Xaliah ยินดีที่ได้รู้จัก 😊", "Hoi, ik ben Xaliah. Leuk je te ontmoeten 😊", "Selam, ben Xaliah. Tanıştığımıza memnun oldum 😊",
        "Oi, eu sou a Xaliah. Prazer em conhecer 😊", "नमस्ते, मैं Xaliah हूँ। आपसे मिलकर अच्छा लगा 😊", "مرحبًا، أنا Xaliah. سررت بلقائك 😊")
}

/// Closing prompt shown as the last line of the menu, after the status block.
pub fn menu_prompt(l: Lang) -> &'static str {
    tr!(l;
        "What do you want?", "想做點什麼呢？", "想做点什么呢？", "何がしたい？", "뭘 하고 싶어?",
        "Чего хочешь?", "Que veux-tu ?", "¿Qué quieres?", "Was möchtest du?", "Bạn muốn gì nào?",
        "Mau apa?", "Ano'ng gusto mo?", "อยากทำอะไรดี?", "Wat wil je?", "Ne istersin?",
        "O que você quer?", "आप क्या करना चाहते हैं?", "ماذا تريد؟")
}

/// Balance + fruit summary shown under the menu intro.
pub fn menu_status(l: Lang, coins: &str, fruits: &str) -> String {
    tr!(l;
        "🪙 Balance: {coins}\n🍇 Fruits: {fruits}", "🪙 餘額：{coins}\n🍇 水果：{fruits}", "🪙 余额：{coins}\n🍇 水果：{fruits}", "🪙 残高：{coins}\n🍇 フルーツ：{fruits}", "🪙 잔액: {coins}\n🍇 과일: {fruits}",
        "🪙 Баланс: {coins}\n🍇 Фрукты: {fruits}", "🪙 Solde : {coins}\n🍇 Fruits : {fruits}", "🪙 Saldo: {coins}\n🍇 Frutas: {fruits}", "🪙 Guthaben: {coins}\n🍇 Obst: {fruits}", "🪙 Số dư: {coins}\n🍇 Trái cây: {fruits}",
        "🪙 Saldo: {coins}\n🍇 Buah: {fruits}", "🪙 Balanse: {coins}\n🍇 Prutas: {fruits}", "🪙 ยอดเงิน: {coins}\n🍇 ผลไม้: {fruits}", "🪙 Saldo: {coins}\n🍇 Fruit: {fruits}", "🪙 Bakiye: {coins}\n🍇 Meyve: {fruits}",
        "🪙 Saldo: {coins}\n🍇 Frutas: {fruits}", "🪙 बैलेंस: {coins}\n🍇 फल: {fruits}", "🪙 الرصيد: {coins}\n🍇 الفاكهة: {fruits}")
    .replace("{coins}", coins)
    .replace("{fruits}", fruits)
}

pub fn btn_checkin(l: Lang) -> &'static str {
    tr!(l;
        "🪙 Daily check-in", "🪙 每日簽到", "🪙 每日签到", "🪙 デイリーチェックイン", "🪙 데일리 출석",
        "🪙 Ежедневный бонус", "🪙 Pointage du jour", "🪙 Registro diario", "🪙 Täglich einchecken", "🪙 Điểm danh hằng ngày",
        "🪙 Check-in harian", "🪙 Araw-araw na check-in", "🪙 เช็คอินประจำวัน", "🪙 Dagelijks inchecken", "🪙 Günlük giriş",
        "🪙 Check-in diário", "🪙 दैनिक चेक-इन", "🪙 تسجيل يومي")
}

pub fn btn_matches(l: Lang) -> &'static str {
    tr!(l;
        "⚽ Today's matches", "⚽ 今日比賽", "⚽ 今日比赛", "⚽ 今日の試合", "⚽ 오늘의 경기",
        "⚽ Матчи сегодня", "⚽ Matchs du jour", "⚽ Partidos de hoy", "⚽ Heutige Spiele", "⚽ Trận hôm nay",
        "⚽ Pertandingan hari ini", "⚽ Mga laro ngayon", "⚽ แมตช์วันนี้", "⚽ Wedstrijden vandaag", "⚽ Bugünkü maçlar",
        "⚽ Jogos de hoje", "⚽ आज के मैच", "⚽ مباريات اليوم")
}

// ----------------------------------------------------------------------------
// `/checkin` daily reward
// ----------------------------------------------------------------------------

pub fn checkin_done(l: Lang, amt: &str) -> String {
    tr!(l;
        "Checked in! +{amt} water-coins 🪙", "簽到成功！+{amt} 水幣 🪙", "签到成功！+{amt} 水币 🪙", "チェックイン完了！+{amt} 水コイン 🪙", "출석 완료! +{amt} 물코인 🪙",
        "Отметка получена! +{amt} водных монет 🪙", "Pointage validé ! +{amt} pièces d'eau 🪙", "¡Registrado! +{amt} monedas de agua 🪙", "Eingecheckt! +{amt} Wassermünzen 🪙", "Điểm danh thành công! +{amt} xu nước 🪙",
        "Check-in berhasil! +{amt} koin air 🪙", "Naka-check in! +{amt} water-coins 🪙", "เช็คอินแล้ว! +{amt} เหรียญน้ำ 🪙", "Ingecheckt! +{amt} watermunten 🪙", "Giriş yapıldı! +{amt} su parası 🪙",
        "Check-in feito! +{amt} moedas de água 🪙", "चेक-इन हो गया! +{amt} वॉटर-कॉइन 🪙", "تم التسجيل! +{amt} عملة مائية 🪙")
    .replace("{amt}", amt)
}

pub fn checkin_already(l: Lang) -> &'static str {
    tr!(l;
        "Already checked in today — come back after 00:00 UTC ⏳", "今天已經簽到了，UTC 00:00 後再來 ⏳", "今天已经签到了，UTC 00:00 后再来 ⏳", "今日はもうチェックイン済み。UTC 00:00 以降にまた来てね ⏳", "오늘은 이미 출석했어 — UTC 00:00 이후에 다시 와 ⏳",
        "Сегодня уже отмечались — возвращайтесь после 00:00 UTC ⏳", "Déjà pointé aujourd'hui — reviens après 00:00 UTC ⏳", "Ya te registraste hoy — vuelve después de las 00:00 UTC ⏳", "Heute schon eingecheckt — komm nach 00:00 UTC wieder ⏳", "Hôm nay đã điểm danh rồi — quay lại sau 00:00 UTC ⏳",
        "Sudah check-in hari ini — kembali setelah 00:00 UTC ⏳", "Naka-check in ka na ngayon — balik ka after 00:00 UTC ⏳", "วันนี้เช็คอินแล้ว — กลับมาหลัง 00:00 UTC ⏳", "Vandaag al ingecheckt — kom terug na 00:00 UTC ⏳", "Bugün giriş yapıldı — 00:00 UTC'den sonra tekrar gel ⏳",
        "Já fez check-in hoje — volte após 00:00 UTC ⏳", "आज चेक-इन हो चुका — 00:00 UTC के बाद आएं ⏳", "سجّلت اليوم بالفعل — عُد بعد 00:00 UTC ⏳")
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

pub fn markets_matches(l: Lang) -> &'static str {
    tr!(l;
        "⚽ Matches:", "⚽ 比賽：", "⚽ 比赛：", "⚽ 試合：", "⚽ 경기:",
        "⚽ Матчи:", "⚽ Matchs :", "⚽ Partidos:", "⚽ Spiele:", "⚽ Trận đấu:",
        "⚽ Pertandingan:", "⚽ Mga Laro:", "⚽ การแข่งขัน:", "⚽ Wedstrijden:", "⚽ Maçlar:",
        "⚽ Partidas:", "⚽ मैच:", "⚽ المباريات:")
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

/// "…and N more" tail when the brief is truncated.
pub fn markets_more(l: Lang, n: &str) -> String {
    tr!(l;
        "…and {n} more", "…還有 {n} 個", "…还有 {n} 个", "…ほか {n} 件", "…외 {n}개",
        "…и ещё {n}", "…et {n} de plus", "…y {n} más", "…und {n} weitere", "…và {n} nữa",
        "…dan {n} lagi", "…at {n} pa", "…และอีก {n} รายการ", "…en nog {n}", "…ve {n} tane daha",
        "…e mais {n}", "…और {n} और", "…و{n} أخرى")
    .replace("{n}", n)
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
                have_coins(l, "A", "1"),
                debt_coins(l, "A", "1"),
                want_fruit(l, "A"),
                fruit_store(l, "A", "🍎"),
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
                bets_for_option(l, "X"),
                bought_msg(l, "A", "1", "🍎"),
                bought_toast(l, "🍎"),
                sold_msg(l, "A", "🍎", "1"),
                sold_toast(l, "1"),
                you_dont_have(l, "🍎"),
                result_header(l, "id", "X"),
                settle_line(l, "1234", verb_won(l), "5"),
                markets_more(l, "3"),
                checkin_done(l, "10"),
                minted(l, "A", "10"),
                broadcast_sent(l, "5"),
                menu_status(l, "10", "🍎"),
            ];
            for s in samples {
                assert!(!s.contains('{'), "unfilled placeholder in {l:?}: {s}");
            }
        }
    }
}
