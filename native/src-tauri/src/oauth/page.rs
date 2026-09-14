//! Rendered HTML pages for OAuth callback responses in Vox.
//!
//! Provides responsive, accessible Light and Dark mode presentation with
//! embedded Vox branding logos (encoded as data URIs) so no external web
//! assets are required.

use std::sync::OnceLock;

const LOGO_DARK_PNG: &[u8] = include_bytes!("../../../src/assets/vox-expanded-dark.png");
const LOGO_LIGHT_PNG: &[u8] = include_bytes!("../../../src/assets/vox-expanded.png");

/// Standard RFC 4648 Base64 encoder without external dependencies.
fn base64_encode(data: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        out.push(CHARSET[(b0 >> 2) as usize] as char);
        out.push(CHARSET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARSET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARSET[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

static SUCCESS_HTML: OnceLock<String> = OnceLock::new();
static ERROR_HTML: OnceLock<String> = OnceLock::new();

/// Returns the rendered HTML response for successful Google authorization.
pub fn render_success_page() -> &'static str {
    SUCCESS_HTML.get_or_init(|| {
        let dark_b64 = base64_encode(LOGO_DARK_PNG);
        let light_b64 = base64_encode(LOGO_LIGHT_PNG);
        render_page(
            true,
            "Connected to Vox",
            "Google authorization was successful. You can close this browser tab and return to the Vox desktop application.",
            &light_b64,
            &dark_b64,
        )
    })
}

/// Returns the rendered HTML response for failed Google authorization.
pub fn render_error_page(message: Option<&str>) -> &'static str {
    ERROR_HTML.get_or_init(|| {
        let dark_b64 = base64_encode(LOGO_DARK_PNG);
        let light_b64 = base64_encode(LOGO_LIGHT_PNG);
        let desc = message.unwrap_or(
            "Google sign-in was canceled, or the OAuth security state was invalid. Please try again in Vox.",
        );
        render_page(
            false,
            "Authorization Failed",
            desc,
            &light_b64,
            &dark_b64,
        )
    })
}

fn render_page(
    is_success: bool,
    title: &str,
    description: &str,
    light_logo_b64: &str,
    dark_logo_b64: &str,
) -> String {
    let (icon_symbol, status_color_light, status_color_dark, badge_bg_light, badge_bg_dark, badge_border_light, badge_border_dark) = if is_success {
        ("✓", "#059669", "#10b981", "#ecfdf5", "rgba(16, 185, 129, 0.12)", "#a7f3d0", "rgba(16, 185, 129, 0.28)")
    } else {
        ("✗", "#dc2626", "#ef4444", "#fef2f2", "rgba(239, 68, 68, 0.12)", "#fecaca", "rgba(239, 68, 68, 0.28)")
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Vox Authorization</title>
  <link rel="icon" type="image/png" href="data:image/png;base64,{dark_logo_b64}">
  <style>
    :root {{
      --bg: #f8fafc;
      --card-bg: #ffffff;
      --card-border: #e2e8f0;
      --card-shadow: 0 20px 40px -15px rgba(0, 0, 0, 0.08), 0 0 1px 1px rgba(0, 0, 0, 0.04);
      --heading: #0f172a;
      --text: #475569;
      --footer: #94a3b8;
      --status: {status_color_light};
      --badge-bg: {badge_bg_light};
      --badge-border: {badge_border_light};
    }}
    @media (prefers-color-scheme: dark) {{
      :root {{
        --bg: #090d16;
        --card-bg: #131b2e;
        --card-border: #1e293b;
        --card-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.7), 0 0 0 1px rgba(255, 255, 255, 0.05);
        --heading: #f8fafc;
        --text: #94a3b8;
        --footer: #64748b;
        --status: {status_color_dark};
        --badge-bg: {badge_bg_dark};
        --badge-border: {badge_border_dark};
      }}
    }}
    * {{
      box-sizing: border-box;
      margin: 0;
      padding: 0;
    }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      background-color: var(--bg);
      color: var(--text);
      transition: background-color 0.2s ease;
      padding: 24px;
    }}
    .auth-card {{
      text-align: center;
      padding: 40px 36px 32px 36px;
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 20px;
      box-shadow: var(--card-shadow);
      max-width: 440px;
      width: 100%;
      animation: popIn 0.35s cubic-bezier(0.16, 1, 0.3, 1);
    }}
    @keyframes popIn {{
      0% {{
        opacity: 0;
        transform: translateY(12px) scale(0.98);
      }}
      100% {{
        opacity: 1;
        transform: translateY(0) scale(1);
      }}
    }}
    .logo-container {{
      display: flex;
      justify-content: center;
      align-items: center;
      margin-bottom: 28px;
    }}
    .vox-logo {{
      height: 38px;
      width: auto;
      object-fit: contain;
      display: block;
    }}
    .status-badge {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      padding: 6px 16px;
      border-radius: 9999px;
      background: var(--badge-bg);
      border: 1px solid var(--badge-border);
      color: var(--status);
      font-size: 18px;
      font-weight: 600;
      letter-spacing: -0.01em;
      margin-bottom: 16px;
    }}
    .status-badge .icon {{
      font-size: 20px;
      line-height: 1;
    }}
    .message {{
      color: var(--text);
      font-size: 14.5px;
      line-height: 1.6;
      margin-bottom: 28px;
    }}
    .footer-tag {{
      font-size: 11px;
      color: var(--footer);
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      letter-spacing: 0.12em;
      text-transform: uppercase;
      border-top: 1px solid var(--card-border);
      padding-top: 20px;
    }}
  </style>
</head>
<body>
  <div class="auth-card">
    <div class="logo-container">
      <picture>
        <source srcset="data:image/png;base64,{dark_logo_b64}" media="(prefers-color-scheme: dark)">
        <img src="data:image/png;base64,{light_logo_b64}" alt="Vox" class="vox-logo">
      </picture>
    </div>
    <div class="status-badge">
      <span class="icon">{icon_symbol}</span>
      <span>{title}</span>
    </div>
    <p class="message">{description}</p>
    <div class="footer-tag">VOX SECURE OAUTH</div>
  </div>
</body>
</html>"#,
        dark_logo_b64 = dark_logo_b64,
        light_logo_b64 = light_logo_b64,
        status_color_light = status_color_light,
        status_color_dark = status_color_dark,
        badge_bg_light = badge_bg_light,
        badge_bg_dark = badge_bg_dark,
        badge_border_light = badge_border_light,
        badge_border_dark = badge_border_dark,
        icon_symbol = icon_symbol,
        title = title,
        description = description
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_render_success_page() {
        let html = render_success_page();
        assert!(html.contains("Connected to Vox"));
        assert!(html.contains("Google authorization was successful."));
        assert!(html.contains("return to the Vox desktop application."));
        assert!(html.contains("VOX SECURE OAUTH"));
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("prefers-color-scheme: dark"));
        assert!(!html.contains("Relay"));
    }

    #[test]
    fn test_render_error_page() {
        let html = render_error_page(None);
        assert!(html.contains("Authorization Failed"));
        assert!(html.contains("Please try again in Vox."));
        assert!(html.contains("VOX SECURE OAUTH"));
        assert!(html.contains("data:image/png;base64,"));
        assert!(!html.contains("Relay"));
    }
}
