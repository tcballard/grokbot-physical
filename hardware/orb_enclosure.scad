// Hey Grok Bot Orb — 110 mm two-piece enclosure
// SPDX-License-Identifier: MIT
//
// Units are millimetres. The default outer diameter matches the concept
// render: a glossy black sphere with a warm halo where it meets the counter.
//
//   openscad -D 'part="lower"'    -o lower.stl    hardware/orb_enclosure.scad
//   openscad -D 'part="upper"'    -o upper.stl    hardware/orb_enclosure.scad
//   openscad -D 'part="chassis"'  -o chassis.stl  hardware/orb_enclosure.scad
//   openscad -D 'part="diffuser"' -o diffuser.stl hardware/orb_enclosure.scad
//
// Print lower and upper. Print chassis flat (standoffs up) and diffuser in
// white or natural PETG. `assembly` is an exploded preview, not a print.
//
// Raise curve_segments to 160 for the shells you actually print. The seam is
// a flush equator. Push the dome on, then twist about 18 degrees until the
// three internal lugs stop. The USB notch is the rear.
//
// Stack, bottom to top:
//   foot, halo slit, NeoPixel ring glued over the slit (LEDs down),
//   3" driver screwed under the chassis with the cone firing up through the
//   acoustic hole, Pi Zero 2 W + ReSpeaker on the standoffs, empty dome.
// The Adafruit 1314 frame is a 77.8 mm square (110 mm on the diagonal). The
// round basket fits this shell; the square tabs do not. Clip the tabs flush
// with the round frame, or set outer_d = 120 before exporting.

/* [Orb] */
outer_d = 110;
wall = 2.6;
foot_inset = 1.5;
curve_segments = 80;

/* [Twist lock] */
n_lugs = 3;
lug_w = 12;
lug_t = 3.2;
skirt_drop = 9.0;
skirt_wall = 2.2;
lock_clearance = 0.40; // increase to 0.6 if the dome will not seat

/* [Halo — Adafruit 1463, 16 LEDs] */
ring_od = 44.5;
ring_id = 31.75;
ring_thickness = 3.2;
diffuser_t = 0.8;

/* [Pi Zero 2 W — hole pitch 58 x 23 mm, M2.5] */
pi_span_x = 58;
pi_span_y = 23;
insert_d = 3.2;     // drill to suit the heat-set inserts you actually have
insert_depth = 5.0;
boss_od = 7.0;
boss_orbit = 45;    // four shell bosses the chassis screws down to
standoff_h = 6;

/* [Speaker — Adafruit 1314, tabs on a 60 mm square] */
speaker_frame = 77.8;
speaker_hole_pitch = 60;
speaker_hole_d = 3.2;
acoustic_d = 52;

/* [Ports] */
usb_w = 12;
usb_h = 7;
mic_hole_d = 1.8;
port_count = 12;
port_hole_d = 2.6;
port_rho = 30;

/* [Which solid to render] */
part = "assembly"; // upper | lower | chassis | diffuser | assembly

$fn = curve_segments;

outer_r = outer_d / 2;
inner_r = outer_r - wall;
chassis_t = 2.4;
chassis_r = 49.5;
chassis_z = -14;   // top of the shell bosses, underside of the chassis
boss_z0 = -34;
acoustic_r = acoustic_d / 2;
window_r_inner = ring_id / 2 - 0.4;
window_r_outer = ring_od / 2 + 0.6;

echo(str("Outer diameter ", outer_d, " mm, inner diameter ", inner_r * 2, " mm"));
speaker_diagonal = speaker_frame * sqrt(2);
echo(str("Speaker frame diagonal ", speaker_diagonal, " mm"));
if (speaker_diagonal > inner_r * 2)
    echo("WARNING: clip the speaker's square tabs, or set outer_d = 120 so the 77.8 mm frame fits.");

module shell() {
    difference() {
        sphere(r = outer_r);
        sphere(r = inner_r);
    }
}

module upper_slab() {
    translate([-outer_r - 5, -outer_r - 5, 0])
        cube([outer_d + 10, outer_d + 10, outer_r + 10]);
}

module lower_slab() {
    translate([-outer_r - 5, -outer_r - 5, -outer_r - 10])
        cube([outer_d + 10, outer_d + 10, outer_r + 10]);
}

module skirt() {
    outer = inner_r - lock_clearance;
    inner = outer - skirt_wall;
    // The sliding skirt sits 0.4 mm inside the bowl. A lip at the equator
    // embeds into the dome wall so the skirt is one piece with the upper shell
    // and still clears the lower rim.
    difference() {
        union() {
            translate([0, 0, -skirt_drop])
                cylinder(h = skirt_drop + 0.6, r = outer);
            cylinder(h = 1.8, r = inner_r + 1.1);
        }
        translate([0, 0, -skirt_drop - 0.1])
            cylinder(h = skirt_drop + 2.2, r = inner);
    }
}

// Entry from the skirt's bottom edge, then a side pocket. A straight push
// seats the lug; the twist carries it fully under the lip so the dome
// cannot lift off.
module skirt_channel() {
    translate([inner_r - 3.3, 0, -skirt_drop / 2])
        cube([7.2, lug_w + 3, skirt_drop + 0.6], center = true);
    translate([inner_r - 3.3, 16, -2.4])
        cube([7.2, 18, lug_t + 1.3], center = true);
}

module lugs() {
    for (i = [0 : n_lugs - 1])
        rotate([0, 0, i * 360 / n_lugs])
            translate([inner_r - 0.5, 0, -2.4])
                cube([3.0, lug_w, lug_t], center = true);
}

module usb_notch() {
    // Rear is -Y. The notch straddles the equator so the cable is trapped
    // in the seam. It passes a lead, not the power-supply plug.
    translate([-usb_w / 2, -outer_r - 1, -usb_h / 2])
        cube([usb_w, wall + 12, usb_h]);
}

module foot_cut() {
    translate([-outer_r - 2, -outer_r - 2, -outer_r - 20])
        cube([outer_d + 4, outer_d + 4, 20 + foot_inset]);
}

module halo_window() {
    // Four narrow bridges keep the flat foot attached to the bowl. A full
    // annular slit would leave the foot as a loose disc.
    difference() {
        translate([0, 0, -outer_r - 1])
            difference() {
                cylinder(h = 16, r = window_r_outer);
                translate([0, 0, -0.2])
                    cylinder(h = 16.4, r = window_r_inner);
            }
        for (k = [0 : 3])
            rotate([0, 0, k * 90 + 20])
                translate([-1.3, window_r_inner - 1, -outer_r - 2])
                    cube([2.6, window_r_outer - window_r_inner + 4, 22]);
    }
}

// A shallow locating channel in the inner skin. The ring is thicker than
// the shell, so the LEDs sit in the cavity and fire down through the slit.
module halo_pocket() {
    intersection() {
        translate([0, 0, -inner_r - 1.0])
            difference() {
                cylinder(h = ring_thickness, r = ring_od / 2 + 0.8);
                translate([0, 0, -0.2])
                    cylinder(h = ring_thickness + 0.4, r = ring_id / 2 - 0.8);
            }
        difference() {
            sphere(r = inner_r + 1.4);
            sphere(r = inner_r - 0.2);
        }
    }
}

module mic_cluster() {
    for (iy = [-1 : 1], iz = [-1 : 1])
        translate([0, iy * 2.4, iz * 2.4])
            rotate([0, 90, 0])
                cylinder(h = 24, d = mic_hole_d, center = true);
}

module mic_ports() {
    // Left and right, just below the seam, so the glossy dome stays closed.
    translate([outer_r - 6, 0, -10])
        mic_cluster();
    rotate([0, 0, 180])
        translate([outer_r - 6, 0, -10])
            mic_cluster();
}

module sound_ports() {
    for (i = [0 : port_count - 1])
        rotate([0, 0, i * 360 / port_count + 8])
            translate([port_rho, 0, -outer_r - 1])
                cylinder(h = 22, d = port_hole_d);
}

module bosses() {
    h = chassis_z - boss_z0;
    intersection() {
        union() {
            for (k = [0 : 3])
                rotate([0, 0, k * 90])
                    translate([boss_orbit, 0, boss_z0])
                        difference() {
                            cylinder(h = h, d = boss_od);
                            translate([0, 0, h - insert_depth])
                                cylinder(h = insert_depth + 0.4, d = insert_d);
                        }
        }
        sphere(r = inner_r + 0.8);
    }
}

module upper() {
    difference() {
        union() {
            intersection() {
                shell();
                upper_slab();
            }
            skirt();
        }
        for (i = [0 : n_lugs - 1])
            rotate([0, 0, i * 360 / n_lugs])
                skirt_channel();
        usb_notch();
    }
}

module lower() {
    difference() {
        union() {
            intersection() {
                shell();
                lower_slab();
            }
            lugs();
            bosses();
        }
        foot_cut();
        halo_window();
        halo_pocket();
        mic_ports();
        sound_ports();
        usb_notch();
    }
}

module pi_standoffs() {
    for (sx = [-1, 1], sy = [-1, 1])
        translate([sx * pi_span_x / 2, sy * pi_span_y / 2, chassis_t - 0.4])
            difference() {
                cylinder(h = standoff_h + 0.4, d = 6.2, $fn = 36);
                translate([0, 0, standoff_h - insert_depth])
                    cylinder(h = insert_depth + 1.2, d = insert_d, $fn = 24);
            }
}

module chassis() {
    difference() {
        union() {
            cylinder(h = chassis_t, r = chassis_r);
            pi_standoffs();
        }
        translate([0, 0, -0.2])
            cylinder(h = chassis_t + 0.4, r = acoustic_r);
        for (sx = [-1, 1], sy = [-1, 1])
            translate([sx * speaker_hole_pitch / 2, sy * speaker_hole_pitch / 2, -0.2])
                cylinder(h = chassis_t + 0.4, d = speaker_hole_d, $fn = 24);
        for (k = [0 : 3])
            rotate([0, 0, k * 90])
                translate([boss_orbit, 0, -0.2])
                    cylinder(h = chassis_t + 0.4, d = 3.4, $fn = 24);
        // Cable drops off the rear edge of the tray toward the notch.
        translate([-4, -chassis_r + 2, -0.2])
            cube([8, 14, chassis_t + 0.4]);
    }
}

module diffuser() {
    difference() {
        cylinder(h = diffuser_t, r = ring_od / 2 + 1.6);
        translate([0, 0, -0.1])
            cylinder(h = diffuser_t + 0.2, r = ring_id / 2 - 1.4);
    }
}

module assembly() {
    color([0.03, 0.03, 0.03])
        lower();
    color([0.08, 0.08, 0.08])
        translate([0, 0, 36])
            upper();
    color([1.0, 0.55, 0.22])
        translate([0, -78, 0])
            diffuser();
    color([0.16, 0.16, 0.17])
        translate([0, 86, 0])
            chassis();
}

if (part == "upper")
    upper();
else if (part == "lower")
    lower();
else if (part == "chassis")
    chassis();
else if (part == "diffuser")
    diffuser();
else if (part == "assembly")
    assembly();
else
    echo(str("Unknown part: ", part));
