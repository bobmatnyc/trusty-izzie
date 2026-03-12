//! Station name to GTFS stop_id lookup for Metro North.

/// Look up a stop ID by station name (case-insensitive, partial match).
pub fn find_stop_id(name: &str) -> Option<u32> {
    let name_lower = name.to_lowercase();
    STATIONS
        .iter()
        .find(|(station_name, _)| station_name.to_lowercase().contains(&name_lower))
        .map(|(_, id)| *id)
}

/// Look up station name by stop ID.
pub fn find_station_name(id: u32) -> Option<&'static str> {
    STATIONS
        .iter()
        .find(|(_, stop_id)| *stop_id == id)
        .map(|(name, _)| *name)
}

/// Core Metro North stations: (name, GTFS stop_id)
pub static STATIONS: &[(&str, u32)] = &[
    ("Grand Central Terminal", 1),
    ("Harlem-125th Street", 4),
    ("Melrose", 5),
    ("Tremont", 6),
    ("Fordham", 7),
    ("Botanical Garden", 8),
    ("Williams Bridge", 9),
    ("Woodlawn", 10),
    ("Wakefield", 11),
    ("Mount Vernon East", 12),
    ("Pelham", 13),
    ("New Rochelle", 14),
    ("Larchmont", 15),
    ("Mamaroneck", 16),
    ("Harrison", 17),
    ("Rye", 18),
    ("Port Chester", 115),
    ("Greenwich", 116),
    ("Cos Cob", 117),
    ("Riverside", 118),
    ("Old Greenwich", 119),
    ("Stamford", 124),
    ("Noroton Heights", 125),
    ("Darien", 126),
    ("Rowayton", 127),
    ("South Norwalk", 128),
    ("East Norwalk", 129),
    ("Westport", 130),
    ("Green's Farms", 131),
    ("Southport", 132),
    ("Fairfield", 133),
    ("Fairfield Metro", 134),
    ("Bridgeport", 135),
    ("Stratford", 136),
    ("Milford", 137),
    ("West Haven", 138),
    ("New Haven State Street", 139),
    ("New Haven", 149),
    ("White Plains", 74),
    ("Poughkeepsie", 39),
    ("Wassaic", 43),
    ("Mount Kisco", 65),
    ("Katonah", 66),
    ("Bedford Hills", 67),
    ("Pleasantville", 70),
    ("Scarsdale", 19),
    ("Tuckahoe", 20),
    ("Bronxville", 21),
    ("Fleetwood", 22),
    ("Mount Vernon West", 23),
    ("Yonkers", 24),
    ("Greystone", 25),
    ("Glenwood", 26),
    ("Hastings-on-Hudson", 27),
    ("Dobbs Ferry", 28),
    ("Ardsley-on-Hudson", 29),
    ("Irvington", 30),
    ("Tarrytown", 31),
    ("Philipse Manor", 32),
    ("Ossining", 33),
    ("Croton-Harmon", 34),
    ("Cortlandt", 35),
    ("Peekskill", 36),
    ("Manitou", 37),
    ("Cold Spring", 38),
    ("Beacon", 39),
];
