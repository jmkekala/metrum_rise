# 	Public Transit

Nothing here is built. It exists so the rail decision is recorded before anyone
implements three separate systems. `roads.md` owns the road network and its
lanes, `traffic.md` owns vehicle movement and routing, and this document owns
the modes and their alignments.

The modes, all first class and all actually running: taxi, bus, tram, train,
subway, light rail, monorail, funicular, gondolas, ferry, blimp, and airplane.
The gondolas means both the aerial cable kind, and the Venice boat kind, used
as public transit.

## Rail is one network with use rules

Tram, subway, light rail, and train are all rail. Real cities run them mixed:
San Francisco's BART, Frankfurt's U-Bahn, Boston's MBTA green line, and
Tokyo's Through-Running Service, are the example to study and contain light
rail on street track downtown and heavy alignment out of town, subways that
surface, trams sharing a corridor with regional services. Modeling them as three
unrelated systems is the mistake that often makes transit in this genre feel like
three toys rather than one cohesive network.

A corridor carries an alignment class and a vehicle carries what it can accept.
What varies with each track is how it is laid and used. A tram needs street-
embedded or shared alignment, a subway needs grade separation, a light rail
needs overhead power, a train needs heavier alignment and longer curves, and
cannot take the tightest street geometry. Which can traverse what track is a
property of the vehicle. Mixed running falls out of the two overlapping, and
so does the refusal: a train will not take a tram's turning radius. That is
the same shape as the lane model in `roads.md`, where a band declares which
modes it admits rather than the network being split per mode.

The other modes stay distinct because they physically are, except arguably
maybe the funicular. Ferries need water and a route across it, gondolas need
towers and span terrain a road cannot, blimps and airplanes need their own
facilities, and funiculars need a slope. Buses and taxis ride the road network
and belong to `roads.md`.

Because the play area is a country rather than a city, the infrastructure is
sized to match: real regional and international airports rather than a token
airfield, and regional hubs that move millions, handling traffic and freight
from around the world.

Freight travels visibly, for as long as it takes. When freight is packed into a
train in the far corner of the map, that train carries it the whole way to its
destination, where it is unloaded and possibly goes somewhere else. A cargo
ship might take an in-game week or two to cross the sea, and the player can
watch it traverse the sea lane the entire time rather than seeing the journey
abstracted into a delivery timer. This is what will make a resource chain
spanning several regions legible and enticing: a chain that is failing shows
the failure in transit rather than only at the destination.
