import glob,gzip,re,collections,os,sys
d=os.path.expanduser("~/Library/Application Support/ikari-ime/logs/")
lines=[]
for f in sorted(glob.glob(d+"engine*.log*")):
    op=gzip.open if f.endswith(".gz") else open
    with op(f,"rt",errors="replace") as fh: lines+=[l for l in fh if " ctx_change " in l]
def kv(l): return dict(x.split("=",1) for x in l.split() if "=" in x)
for name,flt in [("convert,all",lambda k:k.get("req")=="convert"),
  ("convert,dleft==commit",lambda k:k.get("req")=="convert" and k.get("dleft","").lstrip("+")==k.get("commit_chars")),
  ("all,dleft==commit>0",lambda k:k.get("dleft","").lstrip("+")==k.get("commit_chars") and k["commit_chars"]!="0"),
  ("convert,dleft>0,commit>0",lambda k:k.get("req")=="convert" and k.get("dleft","").startswith("+") and k.get("dleft","")!="+0" and k.get("commit_chars","0")!="0")]:
    c=collections.Counter()
    for l in lines:
        k=kv(l)
        if flt(k): c[int(k.get("commit_chars",0))]+=1
    print(name,sum(c.values()),sorted(c.items()))
