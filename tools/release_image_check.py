"""Reject empty renderer captures even when the accessibility tree is ready."""
from PIL import Image


def image_readiness(path):
    with Image.open(path) as source:
        rgba = source.convert('RGBA')
        image = Image.alpha_composite(Image.new('RGBA', rgba.size, 'white'), rgba).convert('RGB')
        image.thumbnail((256, 160))
        spread = max(high - low for low, high in image.getextrema())
        colors = image.getcolors(256)
        color_count = len(colors) if colors is not None else 257
        return dict(nonBlank=spread > 48 and color_count > 16,
                    channelRange=spread, sampledColors=color_count)
