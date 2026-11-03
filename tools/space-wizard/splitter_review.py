#!/usr/bin/env python3
"""Compare both production dividers and drag the mini-app edge in hidden native windows."""
import argparse
import time
from pathlib import Path
from PIL import Image,ImageChops
from member_invite_review import InviteReview
class Review(InviteReview):
 def move(self,x,y):
  self.request('/m',w=0,k='move',x=x,y=y,wait=1);time.sleep(.2)
 def run(self):
  self.request('/event',data='rooms:open-chat');time.sleep(.3)
  self.request('/event',data='rooms:open-apps');time.sleep(.4)
  h=self.widget('handle')['r'];d=self.widget('dock')['r'];p=self.widget('restore_status_view')['r']
  assert h[2]==4,h
  lx=p[0]-2;ly=d[1]+d[3]/2;rx=h[0]+h[2]/2;ry=h[1]+h[3]/2
  self.move(p[0]+100,ly-100);self.capture('idle')
  self.move(lx,ly);self.capture('left-hover')
  self.move(rx,ry);self.capture('right-hover')
  def crop(name,x,y):
   im=Image.open(self.output/(name+'.png')).convert('RGB');scale=im.width/self.width
   return im.crop(tuple(round(v*scale) for v in (x-2,y-24,x+2,y+24)))
  left=crop('left-hover',lx,ly);right=crop('right-hover',rx,ry)
  diff=ImageChops.difference(left,right)
  assert max(hi for lo,hi in diff.getextrema())<=2,('Splitters differ',diff.getextrema())
  assert ImageChops.difference(crop('idle',rx,ry),right).getbbox(),'Right grip must respond to hover'
  self.checks.append('Left and right use pixel-identical centered short hover grips and four-point strips')
  before=self.widget('mini_apps_side')['r'][2]
  # Begin in the same three-point native hit margin used by the left splitter.
  start=h[0]-2;end=start-60
  self.request('/m',w=0,k='down',x=start,y=ry,b=0,wait=1)
  for i in range(1,9):self.request('/m',w=0,k='move',x=start+(end-start)*i/8,y=ry,wait=1)
  self.capture('right-drag')
  self.request('/m',w=0,k='up',x=end,y=ry,b=0,wait=1);time.sleep(.3)
  after=self.widget('mini_apps_side')['r'][2]
  assert abs(after-before-60)<1,(before,after)
  self.request('/event',data='theme:dark' if self.language=='zh-CN' else 'theme:light');time.sleep(.3)
  assert abs(self.widget('mini_apps_side')['r'][2]-after)<1
  self.widget('restore_status_view');self.capture('retained-width')
  self.checks.append('Right divider native hit margin resizes without losing chat or tabs; width survives theme reapply')
def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--binary',type=Path,required=True)
 parser.add_argument('--output',type=Path,required=True)
 args=parser.parse_args()
 for lang,dark in [('zh-CN',False),('en',True)]:
  r=Review(args.binary,args.output/lang,(lang,1440,850,dark,False))
  ok=False
  try:r.run();ok=True
  finally:r.close(ok)

if __name__=='__main__':main()
