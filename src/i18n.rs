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

/// Section heading above the caller's open (unsettled) bets in `/status`.
pub fn positions_title(l: Lang) -> &'static str {
    tr!(l;
        "📊 Open bets", "📊 未結算下注", "📊 未结算下注", "📊 未決済のベット", "📊 미정산 베팅",
        "📊 Открытые ставки", "📊 Paris en cours", "📊 Apuestas abiertas", "📊 Offene Wetten", "📊 Cược đang mở",
        "📊 Taruhan terbuka", "📊 Bukas na taya", "📊 เดิมพันที่เปิดอยู่", "📊 Open weddenschappen", "📊 Açık bahisler",
        "📊 Apostas abertas", "📊 खुले दांव", "📊 الرهانات المفتوحة")
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
pub fn command_menu(l: Lang) -> [(&'static str, &'static str); 5] {
    [
        ("start", menu_start(l)),
        ("balance", menu_balance(l)),
        ("send", menu_send(l)),
        ("predict", menu_predict(l)),
        ("language", menu_language(l)),
    ]
}

fn menu_start(l: Lang) -> &'static str {
    tr!(l;
        "Home page", "首頁", "首页", "ホーム", "홈",
        "Главная", "Accueil", "Inicio", "Startseite", "Trang chính",
        "Beranda", "Home", "หน้าหลัก", "Startpagina", "Ana sayfa",
        "Página inicial", "होम", "الصفحة الرئيسية")
}

fn menu_language(l: Lang) -> &'static str {
    tr!(l;
        "Set language", "設定語言", "设置语言", "言語を設定", "언어 설정",
        "Выбрать язык", "Définir la langue", "Configurar idioma", "Sprache wählen", "Đặt ngôn ngữ",
        "Atur bahasa", "Itakda ang wika", "ตั้งค่าภาษา", "Taal instellen", "Dili ayarla",
        "Definir idioma", "भाषा सेट करें", "تعيين اللغة")
}

fn menu_balance(l: Lang) -> &'static str {
    tr!(l;
        "Asset status", "資產狀況", "资产状况", "資産状況", "자산 현황",
        "Состояние активов", "État des actifs", "Estado de activos", "Vermögensstatus", "Tình trạng tài sản",
        "Status aset", "Estado ng asset", "สถานะสินทรัพย์", "Activastatus", "Varlık durumu",
        "Status dos ativos", "संपत्ति स्थिति", "حالة الأصول")
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
pub fn settle_line(l: Lang, tail: &str, verb: &str, amt: &str) -> String {
    tr!(l;
        "\n***{tail} {verb} {amt} coins", "\n***{tail} {verb}{amt}顆 金幣", "\n***{tail} {verb}{amt}颗 金币", "\n***{tail} コイン{amt}枚{verb}", "\n***{tail} 코인 {amt}개 {verb}",
        "\n***{tail} {verb} {amt} монет", "\n***{tail} {verb} {amt} pièces", "\n***{tail} {verb} {amt} monedas", "\n***{tail} {verb} {amt} Münzen", "\n***{tail} {verb} {amt} xu",
        "\n***{tail} {verb} {amt} koin", "\n***{tail} {verb} {amt} coins", "\n***{tail} {verb} {amt} เหรียญ", "\n***{tail} {verb} {amt} munten", "\n***{tail} {amt} para {verb}",
        "\n***{tail} {verb} {amt} moedas", "\n***{tail} {amt} कॉइन {verb}", "\n***{tail} {verb} {amt} عملة")
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
// Command-usage hints (shown when a command is called with the wrong format)
// ----------------------------------------------------------------------------

pub fn usage_send(l: Lang) -> &'static str {
    tr!(l;
        "Reply to a message, then /send <amount> (or a fruit 🍎)", "回覆一則訊息，然後 /send <數量>（或水果 🍎）", "回复一条消息，然后 /send <数量>（或水果 🍎）", "メッセージに返信して /send <数量>（または果物 🍎）", "메시지에 답장하고 /send <수량> (또는 과일 🍎)",
        "Ответьте на сообщение, затем /send <сумма> (или фрукт 🍎)", "Réponds à un message, puis /send <montant> (ou un fruit 🍎)", "Responde a un mensaje y usa /send <cantidad> (o una fruta 🍎)", "Antworte auf eine Nachricht, dann /send <Betrag> (oder Obst 🍎)", "Trả lời một tin nhắn rồi /send <số lượng> (hoặc trái cây 🍎)",
        "Balas pesan, lalu /send <jumlah> (atau buah 🍎)", "Mag-reply sa mensahe, tapos /send <halaga> (o prutas 🍎)", "ตอบกลับข้อความ แล้ว /send <จำนวน> (หรือผลไม้ 🍎)", "Reageer op een bericht, dan /send <bedrag> (of fruit 🍎)", "Bir mesajı yanıtla, sonra /send <miktar> (veya meyve 🍎)",
        "Responda a uma mensagem e use /send <quantia> (ou uma fruta 🍎)", "किसी संदेश का जवाब दें, फिर /send <राशि> (या फल 🍎)", "ردّ على رسالة ثم /send <المبلغ> (أو فاكهة 🍎)")
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

pub fn usage_predict(l: Lang) -> &'static str {
    tr!(l;
        "/predict <title> <option1> <option2> …", "/predict <標題> <選項1> <選項2> …", "/predict <标题> <选项1> <选项2> …", "/predict <タイトル> <選択肢1> <選択肢2> …", "/predict <제목> <선택지1> <선택지2> …",
        "/predict <название> <вариант1> <вариант2> …", "/predict <titre> <option1> <option2> …", "/predict <título> <opción1> <opción2> …", "/predict <Titel> <Option1> <Option2> …", "/predict <tiêu đề> <lựa chọn1> <lựa chọn2> …",
        "/predict <judul> <opsi1> <opsi2> …", "/predict <pamagat> <opsyon1> <opsyon2> …", "/predict <หัวข้อ> <ตัวเลือก1> <ตัวเลือก2> …", "/predict <titel> <optie1> <optie2> …", "/predict <başlık> <seçenek1> <seçenek2> …",
        "/predict <título> <opção1> <opção2> …", "/predict <शीर्षक> <विकल्प1> <विकल्प2> …", "/predict <العنوان> <خيار1> <خيار2> …")
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
        "💰 Balance", "💰 餘額", "💰 余额", "💰 残高", "💰 잔액",
        "💰 Баланс", "💰 Solde", "💰 Saldo", "💰 Guthaben", "💰 Số dư",
        "💰 Saldo", "💰 Balanse", "💰 ยอดเงิน", "💰 Saldo", "💰 Bakiye",
        "💰 Saldo", "💰 बैलेंस", "💰 الرصيد")
}

pub fn btn_matches(l: Lang) -> &'static str {
    tr!(l;
        "⚽ Today's matches", "⚽ 今日比賽", "⚽ 今日比赛", "⚽ 今日の試合", "⚽ 오늘의 경기",
        "⚽ Матчи сегодня", "⚽ Matchs du jour", "⚽ Partidos de hoy", "⚽ Heutige Spiele", "⚽ Trận hôm nay",
        "⚽ Pertandingan hari ini", "⚽ Mga laro ngayon", "⚽ แมตช์วันนี้", "⚽ Wedstrijden vandaag", "⚽ Bugünkü maçlar",
        "⚽ Jogos de hoje", "⚽ आज के मैच", "⚽ مباريات اليوم")
}

pub fn btn_invite(l: Lang) -> &'static str {
    tr!(l;
        "🔗 Invite friends", "🔗 邀請好友", "🔗 邀请好友", "🔗 友達を招待", "🔗 친구 초대",
        "🔗 Пригласить друзей", "🔗 Inviter des amis", "🔗 Invitar amigos", "🔗 Freunde einladen", "🔗 Mời bạn bè",
        "🔗 Undang teman", "🔗 Mag-imbita ng kaibigan", "🔗 ชวนเพื่อน", "🔗 Vrienden uitnodigen", "🔗 Arkadaş davet et",
        "🔗 Convidar amigos", "🔗 दोस्तों को बुलाएं", "🔗 ادعُ أصدقاءك")
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
pub fn invite_text(l: Lang, link: &str, count: &str) -> String {
    tr!(l;
        "🔗 Invite friends with your link:\n{link}\n\nReferrals: {count} 🎉", "🔗 用你的連結邀請好友：\n{link}\n\n已邀請：{count} 人 🎉", "🔗 用你的链接邀请好友：\n{link}\n\n已邀请：{count} 人 🎉", "🔗 あなたのリンクで友達を招待：\n{link}\n\n招待数：{count} 🎉", "🔗 내 링크로 친구 초대:\n{link}\n\n초대 수: {count} 🎉",
        "🔗 Приглашайте друзей по ссылке:\n{link}\n\nПриглашено: {count} 🎉", "🔗 Invite des amis avec ton lien :\n{link}\n\nParrainages : {count} 🎉", "🔗 Invita amigos con tu enlace:\n{link}\n\nReferidos: {count} 🎉", "🔗 Lade Freunde mit deinem Link ein:\n{link}\n\nEinladungen: {count} 🎉", "🔗 Mời bạn bè bằng liên kết của bạn:\n{link}\n\nĐã mời: {count} 🎉",
        "🔗 Undang teman dengan tautanmu:\n{link}\n\nReferral: {count} 🎉", "🔗 Mag-imbita gamit ang iyong link:\n{link}\n\nReferrals: {count} 🎉", "🔗 ชวนเพื่อนด้วยลิงก์ของคุณ:\n{link}\n\nผู้ที่ชวนมา: {count} 🎉", "🔗 Nodig vrienden uit met je link:\n{link}\n\nVerwijzingen: {count} 🎉", "🔗 Bağlantınla arkadaşlarını davet et:\n{link}\n\nDavetler: {count} 🎉",
        "🔗 Convide amigos com seu link:\n{link}\n\nIndicações: {count} 🎉", "🔗 अपने लिंक से दोस्तों को बुलाएं:\n{link}\n\nरेफ़रल: {count} 🎉", "🔗 ادعُ أصدقاءك عبر رابطك:\n{link}\n\nالإحالات: {count} 🎉")
    .replace("{link}", link)
    .replace("{count}", count)
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

// ----------------------------------------------------------------------------
// Match betting
// ----------------------------------------------------------------------------

pub fn bet_pick(l: Lang) -> &'static str {
    tr!(l;
        "Pick your side (valid 1 min):", "選擇你的下注（1分鐘內有效）：", "选择你的下注（1分钟内有效）：", "賭ける側を選んで（1分間有効）：", "베팅할 쪽을 선택 (1분간 유효):",
        "Выберите сторону (действует 1 мин):", "Choisis ton camp (valable 1 min) :", "Elige tu lado (válido 1 min):", "Wähle deine Seite (1 Min gültig):", "Chọn bên cược (hiệu lực 1 phút):",
        "Pilih sisi taruhanmu (berlaku 1 menit):", "Piliin ang panig (valid 1 min):", "เลือกฝั่งที่จะเดิมพัน (ใช้ได้ 1 นาที):", "Kies je kant (1 min geldig):", "Tarafını seç (1 dk geçerli):",
        "Escolha seu lado (válido 1 min):", "अपना पक्ष चुनें (1 मिनट के लिए मान्य):", "اختر جانبك (صالح لمدة دقيقة):")
}

pub fn bet_unavailable(l: Lang) -> &'static str {
    tr!(l;
        "This match isn't open for betting 😶", "這場比賽無法下注 😶", "这场比赛无法下注 😶", "この試合は賭けられないよ 😶", "이 경기는 베팅할 수 없어 😶",
        "На этот матч ставки закрыты 😶", "Ce match n'est pas ouvert aux paris 😶", "Este partido no está abierto a apuestas 😶", "Auf dieses Spiel kann nicht gewettet werden 😶", "Trận này chưa mở cược 😶",
        "Pertandingan ini belum bisa ditaruhi 😶", "Hindi pa pwedeng pustahan ang laro 😶", "แมตช์นี้ยังเดิมพันไม่ได้ 😶", "Op deze wedstrijd kan niet worden gewed 😶", "Bu maça bahis yapılamıyor 😶",
        "Esta partida não está aberta a apostas 😶", "इस मैच पर दांव नहीं लगा सकते 😶", "هذه المباراة غير متاحة للرهان 😶")
}

pub fn bet_check_dm(l: Lang) -> &'static str {
    tr!(l;
        "Check your DM to place your bet 📩", "請查看私訊來下注 📩", "请查看私信来下注 📩", "DMを確認して賭けてね 📩", "DM에서 베팅을 진행해 📩",
        "Проверьте личные сообщения, чтобы сделать ставку 📩", "Va en privé pour parier 📩", "Revisa tu DM para apostar 📩", "Schau in deine DMs, um zu wetten 📩", "Kiểm tra tin nhắn riêng để đặt cược 📩",
        "Cek DM untuk pasang taruhan 📩", "Tingnan ang DM para tumaya 📩", "ดู DM เพื่อวางเดิมพัน 📩", "Check je DM om te wedden 📩", "Bahis için DM'ine bak 📩",
        "Veja sua DM para apostar 📩", "दांव लगाने के लिए अपना DM देखें 📩", "تحقق من رسائلك الخاصة للمراهنة 📩")
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
        "⌛ Odds changed — open /matches again.", "⌛ 賠率已變動，請重新開啟 /matches。", "⌛ 赔率已变动，请重新打开 /matches。", "⌛ オッズが変わったよ。/matches をもう一度開いてね。", "⌛ 배당이 변경됐어 — /matches 를 다시 열어줘.",
        "⌛ Коэффициенты изменились — откройте /matches снова.", "⌛ Les cotes ont changé — rouvre /matches.", "⌛ Las cuotas cambiaron — abre /matches de nuevo.", "⌛ Quoten geändert — öffne /matches erneut.", "⌛ Tỷ lệ đã thay đổi — mở lại /matches.",
        "⌛ Odds berubah — buka /matches lagi.", "⌛ Nagbago ang odds — buksan ulit ang /matches.", "⌛ อัตราต่อรองเปลี่ยนแล้ว — เปิด /matches อีกครั้ง", "⌛ Odds gewijzigd — open /matches opnieuw.", "⌛ Oranlar değişti — /matches'i tekrar aç.",
        "⌛ As odds mudaram — abra /matches de novo.", "⌛ ऑड्स बदल गए — /matches फिर से खोलें।", "⌛ تغيّرت الاحتمالات — افتح /matches من جديد.")
}

pub fn bet_done(l: Lang) -> &'static str {
    tr!(l;
        "Bet placed ✅", "已下注 ✅", "已下注 ✅", "賭け完了 ✅", "베팅 완료 ✅",
        "Ставка принята ✅", "Pari placé ✅", "Apuesta hecha ✅", "Wette platziert ✅", "Đã đặt cược ✅",
        "Taruhan dipasang ✅", "Tumaya na ✅", "วางเดิมพันแล้ว ✅", "Inzet geplaatst ✅", "Bahis kondu ✅",
        "Aposta feita ✅", "दांव लगा ✅", "تم وضع الرهان ✅")
}

pub fn bet_lost(l: Lang) -> &'static str {
    tr!(l;
        "😔 Your bet didn't win this time.", "😔 這次下注沒中。", "😔 这次下注没中。", "😔 今回は外れたよ。", "😔 이번 베팅은 졌어.",
        "😔 В этот раз ставка не сыграла.", "😔 Ton pari n'a pas gagné cette fois.", "😔 Tu apuesta no ganó esta vez.", "😔 Deine Wette hat diesmal nicht gewonnen.", "😔 Lần này cược của bạn không thắng.",
        "😔 Taruhanmu kali ini kalah.", "😔 Hindi nanalo ang taya mo ngayon.", "😔 ครั้งนี้เดิมพันไม่ชนะ", "😔 Je weddenschap heeft deze keer niet gewonnen.", "😔 Bahsin bu sefer kazanmadı.",
        "😔 Sua aposta não ganhou desta vez.", "😔 इस बार आपका दांव नहीं जीता।", "😔 لم يفز رهانك هذه المرة.")
}

pub fn bet_how_much(l: Lang, side: &str, odds: &str) -> String {
    tr!(l;
        "How much on {side} @ {odds}?", "在 {side} @ {odds} 上下注多少？", "在 {side} @ {odds} 上下注多少？", "{side} @ {odds} にいくら賭ける？", "{side} @ {odds} 에 얼마 걸까?",
        "Сколько на {side} @ {odds}?", "Combien sur {side} @ {odds} ?", "¿Cuánto a {side} @ {odds}?", "Wie viel auf {side} @ {odds}?", "Cược bao nhiêu cho {side} @ {odds}?",
        "Berapa untuk {side} @ {odds}?", "Magkano sa {side} @ {odds}?", "เดิมพันเท่าไรกับ {side} @ {odds}?", "Hoeveel op {side} @ {odds}?", "{side} @ {odds} için ne kadar?",
        "Quanto em {side} @ {odds}?", "{side} @ {odds} पर कितना?", "كم على {side} @ {odds}؟")
    .replace("{side}", side)
    .replace("{odds}", odds)
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

pub fn bet_won(l: Lang, payout: &str) -> String {
    tr!(l;
        "🎉 Your bet won! +{payout} coins", "🎉 你的下注贏了！+{payout} 金幣", "🎉 你的下注赢了！+{payout} 金币", "🎉 賭けに勝ったよ！+{payout} コイン", "🎉 베팅에서 이겼어! +{payout} 코인",
        "🎉 Ваша ставка выиграла! +{payout} монет", "🎉 Ton pari est gagné ! +{payout} pièces", "🎉 ¡Tu apuesta ganó! +{payout} monedas", "🎉 Deine Wette hat gewonnen! +{payout} Münzen", "🎉 Cược của bạn đã thắng! +{payout} xu",
        "🎉 Taruhanmu menang! +{payout} koin", "🎉 Nanalo ang taya mo! +{payout} coins", "🎉 เดิมพันของคุณชนะ! +{payout} เหรียญ", "🎉 Je weddenschap is gewonnen! +{payout} munten", "🎉 Bahsin kazandı! +{payout} para",
        "🎉 Sua aposta ganhou! +{payout} moedas", "🎉 आपका दांव जीत गया! +{payout} कॉइन", "🎉 فاز رهانك! +{payout} عملة")
    .replace("{payout}", payout)
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
                menu_status(l, "10"),
                checkin_already(l, "1h 2m"),
                invite_text(l, "link", "3"),
                referral_bonus(l, "A", "20"),
                bet_how_much(l, "A", "1.54"),
                bet_placed(l, "10", "A", "1.54", "15"),
                bet_won(l, "15"),
            ];
            for s in samples {
                assert!(!s.contains('{'), "unfilled placeholder in {l:?}: {s}");
            }
        }
    }
}
