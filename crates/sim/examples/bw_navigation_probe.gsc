#include maps\mp\bots\_bot_utility;
main()
{
    wps=readWpsFromFile("mp_terminal");bw_event(wps.size==289,"Terminal bundled CSV loader");
    wps=readWpsFromFile("mp_highrise");bw_event(wps.size==422,"Highrise bundled CSV loader");
    wps=readWpsFromFile("mp_afghan");bw_event(wps.size==292,"Afghan bundled CSV loader");
    level.waypoints=[];
    origins[0]=(0,0,0); origins[1]=(100,0,0); origins[2]=(0,120,0); origins[3]=(100,120,0); origins[4]=(1000,1000,0);
    for(i=0;i<5;i++) { w=spawnstruct(); w.origin=origins[i]; w.children=[]; w.type="stand"; level.waypoints[i]=w; }
    level.waypoints[0].children[0]=1;level.waypoints[0].children[1]=2;
    level.waypoints[1].children[0]=3;level.waypoints[2].children[0]=3;
    level.waypointusage["allies"]["1"]=100;
    path=AStarSearch(origins[0],origins[3],"allies",false);
    bw_event(path.size==3,"connected route length");
    bw_event(path[0]==3&&path[1]==2&&path[2]==0,"congestion chooses alternate route");
    path=AStarSearch(origins[3],origins[0],undefined,false);
    bw_event(path.size==0,"directional disconnection returns no route");
    path=AStarSearch(origins[4],origins[0],undefined,false);
    bw_event(path.size==0,"isolated spawn returns no route");
    level.waypoints=[];
    path=AStarSearch(origins[0],origins[3],undefined,false);
    bw_event(path.size==0,"empty waypoint graph returns no route");
}

// Optional diagnostic stress fixture. Intentionally calls a native incorrectly
// 90,000 times to verify fault counting, disk rotation and retained first errors.
diagnostic_flood()
{
    for(frame=0;frame<450;frame++)
    {
        for(i=0;i<200;i++) { self botaction("bad"); }
        wait 0.05;
    }
}
