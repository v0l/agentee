import skrf, numpy as np, sys, warnings; warnings.filterwarnings('ignore')
meas=skrf.Network(sys.argv[1]); sim=skrf.Network(sys.argv[2])
fmax=sim.f[-1]
meas=meas[f'{meas.f[0]}-{fmax}hz'] if False else meas
m=meas.interpolate(skrf.Frequency.from_f(sim.f[sim.f>=meas.f[0]],unit='hz'))
s=sim[sim.f>=meas.f[0]] if False else sim.interpolate(skrf.Frequency.from_f(sim.f[sim.f>=meas.f[0]],unit='hz'))
db=lambda x:20*np.log10(np.abs(x))
print('   f GHz   S21 meas  S21 sim   S11 meas  S11 sim')
for f in [1,2,5,10,15,20,25,30,35,40]:
    i=np.argmin(abs(s.f-f*1e9))
    if abs(s.f[i]-f*1e9)>0.2e9: continue
    print(f'  {f:5.1f}  {db(m.s[i,1,0]):8.3f} {db(s.s[i,1,0]):8.3f}   {db(m.s[i,0,0]):7.1f} {db(s.s[i,0,0]):7.1f}')
def tdr(n, rise=35e-12):
    ne=n.extrapolate_to_dc(kind='linear')
    t,st=ne.s11.step_response(window='hamming',pad=4000)
    return t, 50*(1+st)/(1-st)
for name,n in [('meas',meas),('sim',sim)]:
    t,z=tdr(n)
    sel=(t>0)&(t<1.2e-9)
    t,z=t[sel],z[sel]
    pk=[(round(t[i]*1e12),round(z[i],2)) for i in range(1,len(z)-1) if (z[i]>z[i-1] and z[i]>z[i+1] and abs(z[i]-50)>1.5)]
    mid=z[(t>350e-12)&(t<650e-12)]
    print(name,'Z plateau 350-650 ps: mean',round(mid.mean(),2),'min',round(mid.min(),2),'max',round(mid.max(),2),' peaks',pk[:6])
