using System.Globalization;

namespace Viem.Windows.Editor;

// Transient native form state. Source contains only optional pixel dimensions;
// lock state and the most recently edited field belong to this showing.
internal sealed class ImageSizeDraft
{
    internal bool Expanded { get; private set; }
    internal bool Locked { get; private set; } = true;
    internal string Width { get; private set; } = "";
    internal string Height { get; private set; } = "";
    internal (uint Width, uint Height)? Natural { get; private set; }
    private bool initialAuthoredSize, lockChosen, widthEdited, heightEdited;
    private bool lastEditedHeight;
    private uint? originalWidth, originalHeight;
    private bool sizeChanged, sizeOpened;

    internal void Reset(uint? width, uint? height)
    {
        Expanded = initialAuthoredSize = width != null || height != null;
        sizeOpened = initialAuthoredSize;
        originalWidth = width; originalHeight = height; sizeChanged = false;
        Width = Format(width); Height = Format(height); Natural = null;
        Locked = true; lockChosen = widthEdited = heightEdited = lastEditedHeight = false;
    }

    internal void SetExpanded(bool expanded)
    {
        Expanded = expanded;
        if (expanded && !sizeOpened)
        {
            sizeOpened = sizeChanged = true;
            Locked = true;
            if (Natural != null) FillNatural();
        }
    }

    internal void ClearNatural() => Natural = null;

    internal void SetNatural(uint width, uint height)
    {
        if (width == 0 || height == 0) return;
        Natural = (width, height);
        if (!initialAuthoredSize && !widthEdited && !heightEdited)
        {
            FillNatural(); return;
        }
        uint? authoredWidth = Parse(Width), authoredHeight = Parse(Height);
        if (!lockChosen && !widthEdited && !heightEdited)
            Locked = !initialAuthoredSize || authoredWidth == null || authoredHeight == null
                || Math.Abs(authoredHeight.Value - authoredWidth.Value * (double)height / width) <= 1
                || Math.Abs(authoredWidth.Value - authoredHeight.Value * (double)width / height) <= 1;
        if (!widthEdited && Width.Length == 0)
            Width = authoredHeight is { } h ? Proportion(h, width, height) : Format(width);
        if (!heightEdited && Height.Length == 0)
            Height = Parse(Width) is { } w ? Proportion(w, height, width) : Format(height);
        if (Locked && (widthEdited || heightEdited)) FixRatio();
    }

    internal void EditWidth(string text)
    {
        Width = text; widthEdited = sizeChanged = true; lastEditedHeight = false;
        if (Locked) FixRatio();
    }

    internal void EditHeight(string text)
    {
        Height = text; heightEdited = lastEditedHeight = sizeChanged = true;
        if (Locked) FixRatio();
    }

    internal void SetLocked(bool locked)
    {
        Locked = locked; lockChosen = true;
        string width = Width, height = Height;
        if (locked) FixRatio();
        if (width != Width || height != Height) sizeChanged = true;
    }

    internal void UseNatural()
    {
        if (Natural == null) return;
        sizeChanged = true; FillNatural();
    }

    private void FillNatural()
    {
        if (Natural is not { } natural) return;
        Width = Format(natural.Width); Height = Format(natural.Height);
    }

    internal (uint Width, uint Height) Dimensions()
    {
        // Filling the second display field is presentation only. A location
        // or alt-text edit must preserve a one-axis authored size until the
        // user explicitly changes dimensions.
        if (!sizeChanged) return (originalWidth ?? 0, originalHeight ?? 0);
        if (Width.Length != 0 && Parse(Width) == null || Height.Length != 0 && Parse(Height) == null)
            throw new InvalidOperationException("Enter a positive whole number for the image width or height.");
        return (Parse(Width) ?? 0, Parse(Height) ?? 0);
    }

    private void FixRatio()
    {
        if (Natural is not { } natural) return;
        if (lastEditedHeight)
        {
            if (Parse(Height) is { } height) Width = Proportion(height, natural.Width, natural.Height);
        }
        else if (Parse(Width) is { } width)
            Height = Proportion(width, natural.Height, natural.Width);
    }

    // An unrepresentable derived dimension remains invalid instead of silently
    // clamping and claiming the aspect ratio is still preserved.
    private static string Proportion(uint value, uint numerator, uint denominator)
    {
        double rounded = Math.Max(1, Math.Round(value * (double)numerator / denominator, MidpointRounding.AwayFromZero));
        return rounded.ToString("0", CultureInfo.InvariantCulture);
    }
    private static uint? Parse(string value) => uint.TryParse(value, NumberStyles.None, CultureInfo.InvariantCulture, out uint number) && number > 0 ? number : null;
    private static string Format(uint? value) => value?.ToString(CultureInfo.InvariantCulture) ?? "";
}
