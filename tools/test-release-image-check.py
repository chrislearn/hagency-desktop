#!/usr/bin/env python3
"""Regression: a populated widget tree must not accept an empty GPU capture."""
from pathlib import Path
import tempfile
import unittest
from PIL import Image, ImageDraw
from release_image_check import image_readiness


class RenderedFrameTests(unittest.TestCase):
    def check_image(self, image):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'frame.png'
            image.save(path)
            return image_readiness(path)['nonBlank']

    def test_uniform_clear_frames_are_not_ready(self):
        for color in ['white', 'black', '#e7edf4']:
            with self.subTest(color=color):
                self.assertFalse(self.check_image(Image.new('RGB', (1280, 800), color)))

    def test_transparent_frame_is_not_ready(self):
        self.assertFalse(self.check_image(Image.new('RGBA', (1280, 800), (0, 0, 0, 0))))

    def test_drawn_interface_is_ready(self):
        image = Image.new('RGB', (1280, 800), '#e7edf4')
        draw = ImageDraw.Draw(image)
        draw.rounded_rectangle((300, 220, 980, 590), radius=20, fill='white')
        draw.text((420, 260), 'Sign in to Hagency', fill='#203040', font_size=36)
        draw.rounded_rectangle((420, 460, 860, 520), radius=12, fill='#087080')
        self.assertTrue(self.check_image(image))


if __name__ == '__main__':
    unittest.main()
