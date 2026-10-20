# The oracle for rubyuri: Ruby 2.6's URI.parse, URI#to_s, URI.join and the
# www-form functions. Text is hex of UTF-8 ('-' when empty); 'n' is nil.
require 'uri'

def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  return 'n' if s.nil?
  s = s.to_s
  s.empty? ? '-' : s.b.unpack1('H*')
end

def show(u)
  extras =
    case u
    when URI::FTP then [hx(u.typecode)]
    when URI::LDAP then [u.dn, u.attributes, u.scope, u.filter, u.extensions].map { |x| hx(x) }
    when URI::MailTo then [hx(u.to)] + u.headers.map { |h| 'h' + h.map { |x| hx(x) }.join('.') }
    else []
    end
  [u.class.name.sub('URI::', ''), hx(u.scheme), hx(u.userinfo), hx(u.host), hx(u.port), hx(u.path), hx(u.query), hx(u.fragment),
   hx(u.opaque), extras.join(','), hx(u.to_s)].join('|')
end

def value(t)
  case t[0]
  when 'n' then nil
  when 's' then unhex(t[1..-1])
  else t[1..-1].split('.').map { |x| x == 'n' ? nil : unhex(x) }
  end
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << case f[0]
           when 'pa' then show(URI.parse(unhex(f[1])))
           when 'jo' then show(URI.join(*f[1..-1].map { |x| unhex(x) }))
           when 'ec' then hx(URI.encode_www_form_component(unhex(f[1])))
           when 'dc' then hx(URI.decode_www_form_component(unhex(f[1])))
           when 'ew' then hx(URI.encode_www_form(f[1..-1].map { |kv| k, v = kv.split('=', 2); [unhex(k), value(v)] }))
           else
             r = URI.decode_www_form(unhex(f[1]))
             r.empty? ? '~' : r.map { |k, v| hx(k) + '=' + hx(v) }.join(',')
           end
  rescue StandardError => e
    out << "E:#{e.class}:#{hx(e.message)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
