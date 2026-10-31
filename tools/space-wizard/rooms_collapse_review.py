#!/usr/bin/env python3
"""Native pointer drags through the production HomeScreen and Dock."""
import argparse
import time
from pathlib import Path
from urllib.error import HTTPError
from native_review import Review
CASES={
 'zh-desktop':('zh-CN',1200,800,False,False),
 'en-compact':('en',900,760,False,False),
 'zh-dark':('zh-CN',1200,800,True,False),
 'en-large-dark':('en',1000,800,True,True),
}
class RoomsReview(Review):
    def capture(self,name,window=0):
        for attempt in range(3):
            try:return super().capture(name,window)
            except HTTPError as error:
                if error.code!=404 or attempt==2:raise
                time.sleep(.3)

    def pane(self):
        # With no Matrix login, the real RoomScreen draws its restoring view.
        return self.widget('restore_status_view')

    def rooms_width(self):
        return self.pane()['r'][0]-self.widget('dock')['r'][0]-4

    def drag_to(self,width):
        dock=self.widget('dock')['r']
        start=self.pane()['r'][0]-2
        end=dock[0]+width+2
        y=dock[1]+dock[3]*.65
        self.request('/m',w=0,k='down',x=start,y=y,b=0,wait=1)
        for step in range(1,9):
            self.request('/m',w=0,k='move',x=start+(end-start)*step/8,y=y,wait=1)
        self.request('/m',w=0,k='up',x=end,y=y,b=0,wait=1)
        time.sleep(.15)

    def collapsed(self):
        assert abs(self.rooms_width())<.5,('room list must occupy zero width',self.rooms_width())
        assert not any(w.get('i')=='new_or_join_button' and w.get('r',[0,0,0,0])[2]>0 for w in self.snap()),'Hidden sidebar controls must not retain drawn or clickable fragments'
        assert self.widget('home_button')['r'][2]>0,'Chat navigation must remain reachable'

    def run(self):
        # HomeScreen creates its adaptive page and Dock lazily on first draw.
        # Wait for that real frame before addressing the offline room list.
        for _ in range(50):
            if any(w.get('i')=='rooms_list' and w.get('r',[0,0,0,0])[2]>0 for w in self.snap()):break
            time.sleep(.1)
        else:raise AssertionError('Desktop room list did not render')
        self.request('/event',data='rooms:open-chat')
        time.sleep(.3)
        self.pane()
        assert self.rooms_width()>=220
        self.capture('expanded')
        self.drag_to(-20)
        self.collapsed()
        self.capture('collapsed')
        self.request('/event',data='rooms:reload')
        time.sleep(.25)
        self.collapsed()
        self.pane()
        self.checks.append('Drag fully left leaves no sidebar fragments; saved collapsed layout reloads without losing the chat')
        self.drag_to(280)
        assert abs(self.rooms_width()-280)<2,self.rooms_width()
        self.widget('new_or_join_button')
        self.capture('drag-restored')
        self.drag_to(180)
        assert self.rooms_width()>=220,'Expanded sidebar keeps a usable minimum width'
        self.drag_to(-20)
        self.collapsed()
        self.click('home_button')
        assert abs(self.rooms_width()-300)<2,self.rooms_width()
        self.widget('new_or_join_button')
        self.pane()
        self.capture('home-restored')
        self.checks.append('Boundary drag and Chats button both restore the sidebar and retain the current conversation')
        self.drag_to(-20)
        self.request('/event',data='theme:light' if self.case_dark else 'theme:dark')
        time.sleep(.2)
        self.collapsed()
        self.click('home_button')
        assert self.rooms_width()>=220
        self.checks.append('Theme reapply preserves collapsed state and recovery controls')
        log=(self.output/'native.log').read_text()
        assert 'panicked at' not in log and 'Expected expression' not in log

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--case',choices=CASES,required=True)
    args=parser.parse_args()
    review=RoomsReview(args.binary,args.output,CASES[args.case])
    review.case_dark=CASES[args.case][3]
    passed=False
    try:
        review.run();passed=True
    finally:review.close(passed)
if __name__=='__main__':main()
