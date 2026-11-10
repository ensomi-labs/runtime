"""Checks that can fail on mistakes in the decision procedure, independent of ONNX."""
import unittest
from run import search, structural, grid_edges, arm_options

class DriverTests(unittest.TestCase):
    def test_exact_first_try_stops(self):
        result=search(lambda r,s:{'star':r},4,1000,'secant')
        self.assertEqual(result['generations'],1)
        self.assertEqual(result['error'],0)

    def test_plateau_moves_to_seeds_and_keeps_earliest_tie(self):
        calls=[]
        def generate(r,s):calls.append((r,s));return {'star':2.5}
        result=search(generate,1,1000,'secant')
        self.assertEqual(calls,[(1,1000),(-.5,1000),(1,1001),(1,1002),(1,1003),(1,1004)])
        self.assertTrue(result['saturated'])
        self.assertEqual(result['best']['k'],0)

    def test_secant_estimates_a_known_slope(self):
        result=search(lambda r,s:{'star':1.5*r},4,0,'secant')
        self.assertAlmostEqual(result['best']['r'],4/1.5)
        self.assertEqual(result['generations'],3)

    def test_decreasing_response_is_bounded_and_best_cannot_worsen(self):
        result=search(lambda r,s:{'star':8-r},3,0,'secant')
        self.assertLessEqual(result['error'],result['first_error'])
        self.assertTrue(all(0<=t['r']<=6 for t in result['tries']))
        self.assertEqual(result['generations'],6)

    def test_fixed_seeds_uses_its_entire_budget(self):
        result=search(lambda r,s:{'star':r},4,0,'seeds')
        self.assertEqual([t['seed'] for t in result['tries']],list(range(6)))
        self.assertEqual(result['best']['k'],0)

    def test_quantile_endpoints_are_exactly_the_tested_values(self):
        self.assertEqual(arm_options('q_tested',1)['difficulty_q'],.1)
        self.assertEqual(arm_options('q_tested',9)['difficulty_q'],.75)

    def test_arms_are_off_inside_fitted_interval(self):
        for arm in ['overall','end_segment','q_tested','q_outer','direct','ceiling','masks','overall+direct']:
            for star in [2.125,2.5,4,5.875]:self.assertEqual(arm_options(arm,star),{})

    def test_chord_wall_and_unused_lanes_are_visible(self):
        wall=structural([(lane,t,None) for t in [0,100,200] for lane in range(4)],1000,[[0,500,4]])
        self.assertEqual(wall['quads'],1)
        self.assertEqual(wall['min_reattack'],100)
        one=structural([(0,t,None) for t in [0,100,200]],1000,[[0,500,4]])
        self.assertEqual(one['lane_max'],1)
        self.assertEqual(one['lanes_used'],1)

    def test_rounded_head_stays_in_its_intended_beat(self):
        m=structural([(i,t,None) for i,t in enumerate([0,667,1333,2000])],2666.6667,[[0,2000/3,4]])
        # ceil(duration/beat) includes a fifth partial beat for this slightly longer duration.
        self.assertAlmostEqual(m['empty_beats'],.2)

if __name__=='__main__':unittest.main()
