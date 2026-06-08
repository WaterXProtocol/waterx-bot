use crate::bot::ParamsKey;
use crate::commands::util::*;
use telexide::prelude::*;

#[command(description = "owner-only: view or mutate runtime params (p_possi/p_mean/p_std)")]
pub async fn param(ctx: Context, message: Message) -> CommandResult {
    let Some(uid) = from_id(&message) else {
        return Ok(());
    };
    if !is_owner(&ctx, uid) {
        return Ok(());
    }
    let parts = args(&message);
    let params = ctx
        .data
        .read()
        .get::<ParamsKey>()
        .expect("ParamsKey missing")
        .clone();

    if parts.is_empty() {
        // Dump everything p_*
        let txt = {
            let p = params.read();
            format!(
                "p_possi = {}\np_mean = {}\np_std = {}",
                p.p_possi, p.p_mean, p.p_std
            )
        };
        reply(&ctx, &message, txt).await?;
        return Ok(());
    }

    let key = parts[0].as_str();
    if parts.len() == 1 {
        let txt = {
            let p = params.read();
            match key {
                "p_possi" => format!("p_possi = {}", p.p_possi),
                "p_mean" => format!("p_mean = {}", p.p_mean),
                "p_std" => format!("p_std = {}", p.p_std),
                _ => "no such param".into(),
            }
        };
        reply(&ctx, &message, txt).await?;
        return Ok(());
    }

    // parts.len() >= 2 → set value
    if !key.starts_with("p_") {
        reply(&ctx, &message, "command error").await?;
        return Ok(());
    }
    let val_str = &parts[1];
    let txt = {
        let mut p = params.write();
        match key {
            "p_possi" => match val_str.parse::<u32>() {
                Ok(v) if v >= 1 => {
                    p.p_possi = v;
                    format!("p_possi = {v}")
                }
                Ok(_) => "p_possi must be >= 1".into(),
                Err(_) => "parse error".into(),
            },
            "p_mean" => match val_str.parse::<f64>() {
                Ok(v) if v.is_finite() => {
                    p.p_mean = v;
                    format!("p_mean = {v}")
                }
                Ok(_) => "p_mean must be finite".into(),
                Err(_) => "parse error".into(),
            },
            "p_std" => match val_str.parse::<f64>() {
                Ok(v) if v.is_finite() && v >= 0.0 => {
                    p.p_std = v;
                    format!("p_std = {v}")
                }
                Ok(_) => "p_std must be finite and >= 0".into(),
                Err(_) => "parse error".into(),
            },
            _ => "no such param".into(),
        }
    };
    reply(&ctx, &message, txt).await?;
    Ok(())
}
