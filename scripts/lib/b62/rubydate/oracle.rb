# The oracle for rubydate: Ruby 2.6's Date (date 2.0.x in ext/date).
# A start is a Julian day number, or I, E, J, G for ITALY, ENGLAND,
# JULIAN and GREGORIAN.
require 'date'
$VERBOSE = nil

def sg(t)
  { 'I' => Date::ITALY, 'E' => Date::ENGLAND, 'J' => Date::JULIAN, 'G' => Date::GREGORIAN }[t] || Integer(t)
end

def show(d)
  [d.inspect, d.year, d.mon, d.mday, d.wday, d.yday, d.cwyear, d.cweek, d.cwday, d.jd, d.mjd, d.ld, d.ajd.inspect, d.amjd.inspect,
   d.leap?, d.julian?, d.gregorian?, d.start].join('|')
end

def run(f)
  i = ->(k) { Integer(f[k]) }
  case f[0]
  when 'vc' then Date.valid_civil?(i[1], i[2], i[3], sg(f[4])).to_s
  when 'vo' then Date.valid_ordinal?(i[1], i[2], sg(f[3])).to_s
  when 'vm' then Date.valid_commercial?(i[1], i[2], i[3], sg(f[4])).to_s
  when 'cv' then show(Date.civil(i[1], i[2], i[3], sg(f[4])))
  when 'jd' then show(Date.jd(i[1], sg(f[2])))
  when 'od' then show(Date.ordinal(i[1], i[2], sg(f[3])))
  when 'cm' then show(Date.commercial(i[1], i[2], i[3], sg(f[4])))
  when 'sh' then show(Date.civil(i[1], i[2], i[3], sg(f[4])) >> i[5])
  when 'ls' then show(Date.civil(i[1], i[2], i[3], sg(f[4])) << i[5])
  when 'ad' then show(Date.civil(i[1], i[2], i[3], sg(f[4])) + i[5])
  when 'ns' then show(Date.civil(i[1], i[2], i[3], sg(f[4])).new_start(sg(f[5])))
  when 'df'
    a = Date.civil(i[1], i[2], i[3], sg(f[4]))
    b = Date.civil(i[5], i[6], i[7], sg(f[8]))
    "#{(a - b).inspect}|#{(a <=> b).inspect}"
  end
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << run(f)
  rescue ArgumentError => e
    out << "E:#{e.class}:#{e.message.unpack1('H*')}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
