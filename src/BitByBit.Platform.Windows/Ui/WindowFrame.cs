using System.Runtime.Versioning;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.Dwm;

namespace BitByBit.Platform.Windows.Ui;

/// <summary>
/// Makes the system title bar follow the app's look (dark/light caption, caption and border colours). Each attribute is
/// optional: E_INVALIDARG from older Windows builds just means "not supported here" and is ignored.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public static class WindowFrame
{
    private const int UseImmersiveDarkModeLegacy = 19; // Windows 10 1809–1903

    /// <param name="caption">Caption colour as 0xRRGGBB, or null to keep the system colour.</param>
    /// <param name="text">Caption text colour as 0xRRGGBB, or null.</param>
    /// <param name="border">Window border colour as 0xRRGGBB, or null.</param>
    /// <returns>Short English summary of what Windows accepted (for the log).</returns>
    public static unsafe string Apply(nint hwnd, bool dark, int? caption, int? text, int? border)
    {
        if (hwnd == 0)
        {
            return "no window";
        }

        var handle = new HWND(hwnd);
        int darkValue = dark ? 1 : 0;
        var applied = new List<string>();
        if (PInvoke.DwmSetWindowAttribute(handle, DWMWINDOWATTRIBUTE.DWMWA_USE_IMMERSIVE_DARK_MODE, &darkValue, sizeof(int)).Succeeded ||
            PInvoke.DwmSetWindowAttribute(handle, (DWMWINDOWATTRIBUTE)UseImmersiveDarkModeLegacy, &darkValue, sizeof(int)).Succeeded)
        {
            applied.Add(dark ? "dark mode" : "light mode");
        }

        void SetColor(DWMWINDOWATTRIBUTE attribute, int? rgb, string name)
        {
            if (rgb is not { } value)
            {
                return;
            }

            // COLORREF is 0x00BBGGRR.
            var colorRef = ((value & 0xFF) << 16) | (value & 0xFF00) | ((value >> 16) & 0xFF);
            if (PInvoke.DwmSetWindowAttribute(handle, attribute, &colorRef, sizeof(int)).Succeeded)
            {
                applied.Add(name);
            }
        }

        SetColor(DWMWINDOWATTRIBUTE.DWMWA_CAPTION_COLOR, caption, "caption");
        SetColor(DWMWINDOWATTRIBUTE.DWMWA_TEXT_COLOR, text, "text");
        SetColor(DWMWINDOWATTRIBUTE.DWMWA_BORDER_COLOR, border, "border");

        // Without a frame change the title bar only repaints on the next unrelated redraw.
        PInvoke.SetWindowPos(handle, HWND.Null, 0, 0, 0, 0,
            global::Windows.Win32.UI.WindowsAndMessaging.SET_WINDOW_POS_FLAGS.SWP_NOMOVE | global::Windows.Win32.UI.WindowsAndMessaging.SET_WINDOW_POS_FLAGS.SWP_NOSIZE |
            global::Windows.Win32.UI.WindowsAndMessaging.SET_WINDOW_POS_FLAGS.SWP_NOZORDER | global::Windows.Win32.UI.WindowsAndMessaging.SET_WINDOW_POS_FLAGS.SWP_NOACTIVATE |
            global::Windows.Win32.UI.WindowsAndMessaging.SET_WINDOW_POS_FLAGS.SWP_FRAMECHANGED);
        return applied.Count == 0 ? "title bar colours not supported" : string.Join(", ", applied);
    }
}
