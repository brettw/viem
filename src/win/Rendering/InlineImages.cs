namespace Viem.Windows.Rendering;

/// `Measured`: intrinsic dimensions are known from the file header, but no
/// bitmap is decoded. Layout uses them exactly, like `Ready`.
internal enum InlineImageState { Pending, Measured, Ready, Broken, Limited }

/// What layout knows about one image location. A change to its layout key
/// requires reshaping; anything else (a decoded bitmap, a broken indicator)
/// only repaints.
internal sealed class InlineImageKnowledge
{
    public uint Width = 300, Height = 64;
    public InlineImageState State;
    /// Whether layout has used these dimensions since they were recorded.
    public bool Served;
    public bool Exact => State is InlineImageState.Measured or InlineImageState.Ready;
    public (uint Width, uint Height, bool Exact) LayoutKey => (Width, Height, Exact);
    public InlineImageKnowledge Copy() => new() { Width = Width, Height = Height, State = State, Served = Served };
}
